use std::{collections::VecDeque, fmt, sync::Arc};

use parking_lot::Mutex;
use tokio::sync::Notify;

use crate::domain::{
    ActiveOperationKind, ActiveOperationSummaryDto, MAX_SAFE_ACTIVE_OPERATION_COUNT,
    MaintenanceErrorCode, MaintenanceErrorDto, MaintenanceStatusCode, MaintenanceStatusDto,
};

#[derive(Clone)]
pub struct MaintenanceGate {
    inner: Arc<GateInner>,
}

struct GateInner {
    state: Mutex<GateState>,
    changed: Notify,
}

#[derive(Default)]
struct GateState {
    active: [u32; ActiveOperationKind::ALL.len()],
    waiting_maintenance: VecDeque<u64>,
    next_ticket: u64,
    exclusive: bool,
    shutting_down: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateAcquireError {
    Busy(MaintenanceStatusDto),
    ShuttingDown,
    CapacityExceeded,
}

impl GateAcquireError {
    pub const fn as_app_error_code(&self) -> crate::errors::AppErrorCode {
        crate::errors::AppErrorCode::RequestConflict
    }
}

impl From<GateAcquireError> for crate::errors::AppError {
    fn from(error: GateAcquireError) -> Self {
        crate::errors::AppError::new(error.as_app_error_code())
    }
}

impl From<GateAcquireError> for MaintenanceErrorDto {
    fn from(error: GateAcquireError) -> Self {
        match error {
            GateAcquireError::Busy(status) => Self::busy(status),
            GateAcquireError::ShuttingDown => {
                Self::new(MaintenanceErrorCode::MaintenanceShuttingDown)
            }
            GateAcquireError::CapacityExceeded => {
                Self::new(MaintenanceErrorCode::MaintenanceCapacityExceeded)
            }
        }
    }
}

impl Default for MaintenanceGate {
    fn default() -> Self {
        Self {
            inner: Arc::new(GateInner {
                state: Mutex::new(GateState::default()),
                changed: Notify::new(),
            }),
        }
    }
}

impl fmt::Debug for MaintenanceGate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MaintenanceGate")
            .field("status", &self.status())
            .finish()
    }
}

impl MaintenanceGate {
    pub fn try_acquire_normal(
        &self,
        kind: ActiveOperationKind,
    ) -> Result<NormalOperationPermit, GateAcquireError> {
        let mut state = self.inner.state.lock();
        if state.shutting_down {
            return Err(GateAcquireError::ShuttingDown);
        }
        if state.exclusive || !state.waiting_maintenance.is_empty() {
            return Err(GateAcquireError::Busy(snapshot(&state)));
        }
        increment_active(&mut state, kind)?;
        Ok(NormalOperationPermit {
            inner: Some(self.inner.clone()),
            kind,
        })
    }

    pub async fn acquire_normal(
        &self,
        kind: ActiveOperationKind,
    ) -> Result<NormalOperationPermit, GateAcquireError> {
        loop {
            let notified = self.inner.changed.notified();
            {
                let mut state = self.inner.state.lock();
                if state.shutting_down {
                    return Err(GateAcquireError::ShuttingDown);
                }
                if !state.exclusive && state.waiting_maintenance.is_empty() {
                    increment_active(&mut state, kind)?;
                    return Ok(NormalOperationPermit {
                        inner: Some(self.inner.clone()),
                        kind,
                    });
                }
            }
            notified.await;
        }
    }

    pub fn try_acquire_maintenance(&self) -> Result<ExclusiveMaintenancePermit, GateAcquireError> {
        let mut state = self.inner.state.lock();
        if state.shutting_down {
            return Err(GateAcquireError::ShuttingDown);
        }
        if state.exclusive || !state.waiting_maintenance.is_empty() || active_total(&state)? > 0 {
            return Err(GateAcquireError::Busy(snapshot(&state)));
        }
        state.exclusive = true;
        Ok(ExclusiveMaintenancePermit {
            inner: Some(self.inner.clone()),
        })
    }

    pub async fn acquire_maintenance(
        &self,
    ) -> Result<ExclusiveMaintenancePermit, GateAcquireError> {
        let mut waiter = QueuedMaintenance::register(self.inner.clone())?;
        loop {
            let notified = self.inner.changed.notified();
            {
                let mut state = self.inner.state.lock();
                if state.shutting_down {
                    return Err(GateAcquireError::ShuttingDown);
                }
                if !state.exclusive
                    && active_total(&state)? == 0
                    && state.waiting_maintenance.front() == Some(&waiter.ticket)
                {
                    state.waiting_maintenance.pop_front();
                    state.exclusive = true;
                    waiter.claimed = true;
                    return Ok(ExclusiveMaintenancePermit {
                        inner: Some(self.inner.clone()),
                    });
                }
            }
            notified.await;
        }
    }

    pub fn status(&self) -> MaintenanceStatusDto {
        snapshot(&self.inner.state.lock())
    }

    pub fn shutdown(&self) {
        {
            let mut state = self.inner.state.lock();
            state.shutting_down = true;
        }
        self.inner.changed.notify_waiters();
    }
}

#[must_use = "dropping the permit releases the normal-operation slot"]
pub struct NormalOperationPermit {
    inner: Option<Arc<GateInner>>,
    kind: ActiveOperationKind,
}

impl NormalOperationPermit {
    pub fn release(&mut self) {
        let Some(inner) = self.inner.take() else {
            return;
        };
        {
            let mut state = inner.state.lock();
            let active = &mut state.active[self.kind.index()];
            if *active > 0 {
                *active -= 1;
            }
        }
        inner.changed.notify_waiters();
    }
}

impl fmt::Debug for NormalOperationPermit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NormalOperationPermit")
            .field("kind", &self.kind)
            .field("released", &self.inner.is_none())
            .finish()
    }
}

impl Drop for NormalOperationPermit {
    fn drop(&mut self) {
        self.release();
    }
}

#[must_use = "dropping the permit ends exclusive maintenance"]
pub struct ExclusiveMaintenancePermit {
    inner: Option<Arc<GateInner>>,
}

impl ExclusiveMaintenancePermit {
    pub fn release(&mut self) {
        let Some(inner) = self.inner.take() else {
            return;
        };
        {
            let mut state = inner.state.lock();
            state.exclusive = false;
        }
        inner.changed.notify_waiters();
    }
}

impl fmt::Debug for ExclusiveMaintenancePermit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExclusiveMaintenancePermit")
            .field("released", &self.inner.is_none())
            .finish()
    }
}

impl Drop for ExclusiveMaintenancePermit {
    fn drop(&mut self) {
        self.release();
    }
}

struct QueuedMaintenance {
    inner: Arc<GateInner>,
    ticket: u64,
    claimed: bool,
}

impl QueuedMaintenance {
    fn register(inner: Arc<GateInner>) -> Result<Self, GateAcquireError> {
        let ticket = {
            let mut state = inner.state.lock();
            if state.shutting_down {
                return Err(GateAcquireError::ShuttingDown);
            }
            let ticket = state.next_ticket;
            state.next_ticket = state
                .next_ticket
                .checked_add(1)
                .ok_or(GateAcquireError::CapacityExceeded)?;
            state.waiting_maintenance.push_back(ticket);
            ticket
        };
        inner.changed.notify_waiters();
        Ok(Self {
            inner,
            ticket,
            claimed: false,
        })
    }
}

impl Drop for QueuedMaintenance {
    fn drop(&mut self) {
        if self.claimed {
            return;
        }
        {
            let mut state = self.inner.state.lock();
            state
                .waiting_maintenance
                .retain(|candidate| *candidate != self.ticket);
        }
        self.inner.changed.notify_waiters();
    }
}

fn increment_active(
    state: &mut GateState,
    kind: ActiveOperationKind,
) -> Result<(), GateAcquireError> {
    state.active[kind.index()] = state.active[kind.index()]
        .checked_add(1)
        .ok_or(GateAcquireError::CapacityExceeded)?;
    Ok(())
}

fn active_total(state: &GateState) -> Result<u32, GateAcquireError> {
    state
        .active
        .iter()
        .try_fold(0_u32, |total, count| total.checked_add(*count))
        .ok_or(GateAcquireError::CapacityExceeded)
}

fn snapshot(state: &GateState) -> MaintenanceStatusDto {
    let active_operations = ActiveOperationKind::ALL
        .into_iter()
        .filter_map(|kind| {
            let count = state.active[kind.index()];
            (count > 0).then_some(ActiveOperationSummaryDto {
                kind,
                count: count.min(MAX_SAFE_ACTIVE_OPERATION_COUNT),
            })
        })
        .collect();
    let code = if state.shutting_down {
        MaintenanceStatusCode::MaintenanceShuttingDown
    } else if state.exclusive {
        MaintenanceStatusCode::MaintenanceExclusive
    } else if !state.waiting_maintenance.is_empty() {
        MaintenanceStatusCode::MaintenanceWaiting
    } else if state.active.iter().any(|count| *count > 0) {
        MaintenanceStatusCode::NormalOperationsActive
    } else {
        MaintenanceStatusCode::MaintenanceAvailable
    };
    MaintenanceStatusDto {
        code,
        active_operations,
    }
}

#[cfg(test)]
#[path = "gate_test.rs"]
mod tests;
