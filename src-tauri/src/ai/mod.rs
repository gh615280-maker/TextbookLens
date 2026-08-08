pub mod error;
pub(crate) mod kimi_region;
pub mod multimodal;
pub mod provider;
pub mod providers;
pub mod registry;
pub mod runtime;
pub mod stream;
pub mod structured;
pub mod transport;

#[cfg(test)]
mod multimodal_test;
#[cfg(test)]
mod operation_contract_test;
#[cfg(test)]
mod provider_contract_test;
#[cfg(test)]
mod registry_test;
#[cfg(test)]
mod runtime_test;
#[cfg(test)]
mod stream_test;
#[cfg(test)]
mod structured_test;
#[cfg(test)]
mod transport_test;
