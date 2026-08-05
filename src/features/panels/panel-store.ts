import type { PanelGeometry } from '../../lib/generated/panel';
import {
  DEFAULT_PANEL_GEOMETRY,
  normalizePanelGeometry,
} from './panel-geometry';

export interface FloatingPanel {
  readonly id: string;
  readonly requestId: string;
  readonly conversationId: string | null;
  readonly hidden: boolean;
  readonly collapsed: boolean;
  readonly zIndex: number;
  readonly geometry: Readonly<PanelGeometry>;
}

export interface FloatingPanelState {
  readonly panels: readonly FloatingPanel[];
}

export class PanelStore {
  private readonly panels = new Map<string, FloatingPanel>();
  private readonly listeners = new Set<() => void>();
  private nextId = 1;
  private nextZ = 1;

  snapshot(): FloatingPanelState {
    return Object.freeze({
      panels: Object.freeze(
        [...this.panels.values()].sort(
          (left, right) => left.zIndex - right.zIndex,
        ),
      ),
    });
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  ensureRequest(
    requestId: string,
    geometry = DEFAULT_PANEL_GEOMETRY,
  ): FloatingPanel {
    const existing = this.byRequest(requestId);
    if (existing) return existing;
    const panel = freezePanel({
      id: `learning-panel-${this.nextId++}`,
      requestId,
      conversationId: null,
      hidden: false,
      collapsed: false,
      zIndex: this.nextZ++,
      geometry: normalizePanelGeometry(geometry),
    });
    this.panels.set(panel.id, panel);
    this.emit();
    return panel;
  }

  completeRequest(
    requestId: string,
    conversationId: string,
  ): FloatingPanel | null {
    const pending = this.byRequest(requestId);
    if (!pending) return null;
    const existing = this.byConversation(conversationId);
    if (existing && existing.id !== pending.id) {
      this.panels.delete(pending.id);
      return this.reopen(existing.id);
    }
    return this.replace(pending.id, { conversationId });
  }

  openConversation(
    conversationId: string,
    geometry = DEFAULT_PANEL_GEOMETRY,
  ): FloatingPanel {
    const existing = this.byConversation(conversationId);
    if (existing) return this.reopen(existing.id);
    const panel = freezePanel({
      id: `learning-panel-${this.nextId++}`,
      requestId: `history:${conversationId}`,
      conversationId,
      hidden: false,
      collapsed: false,
      zIndex: this.nextZ++,
      geometry: normalizePanelGeometry(geometry),
    });
    this.panels.set(panel.id, panel);
    this.emit();
    return panel;
  }

  attachRequestToConversation(
    requestId: string,
    conversationId: string,
  ): FloatingPanel {
    const conversationPanel = this.byConversation(conversationId);
    const requestPanel = this.byRequest(requestId);
    if (conversationPanel) {
      if (requestPanel && requestPanel.id !== conversationPanel.id) {
        this.panels.delete(requestPanel.id);
      }
      return this.replace(conversationPanel.id, {
        requestId,
        hidden: false,
        zIndex: this.nextZ++,
      });
    }
    const created = this.openConversation(conversationId);
    return this.replace(created.id, { requestId });
  }

  removeConversation(conversationId: string): boolean {
    const panel = this.byConversation(conversationId);
    if (!panel) return false;
    this.panels.delete(panel.id);
    this.emit();
    return true;
  }

  reopen(id: string): FloatingPanel {
    return this.replace(id, { hidden: false, zIndex: this.nextZ++ });
  }

  hide(id: string): FloatingPanel {
    return this.replace(id, { hidden: true });
  }

  setCollapsed(id: string, collapsed: boolean): FloatingPanel {
    return this.replace(id, { collapsed });
  }

  setGeometry(id: string, geometry: PanelGeometry): FloatingPanel {
    return this.replace(id, { geometry: normalizePanelGeometry(geometry) });
  }

  private replace(id: string, changes: Partial<FloatingPanel>): FloatingPanel {
    const current = this.panels.get(id);
    if (!current) throw new Error('Unknown floating panel');
    const next = freezePanel({ ...current, ...changes });
    this.panels.set(id, next);
    this.emit();
    return next;
  }

  private byRequest(requestId: string): FloatingPanel | undefined {
    return [...this.panels.values()].find(
      (panel) => panel.requestId === requestId,
    );
  }

  private byConversation(conversationId: string): FloatingPanel | undefined {
    return [...this.panels.values()].find(
      (panel) => panel.conversationId === conversationId,
    );
  }

  private emit() {
    for (const listener of this.listeners) listener();
  }
}

function freezePanel(panel: FloatingPanel): FloatingPanel {
  return Object.freeze({
    ...panel,
    geometry: normalizePanelGeometry(panel.geometry),
  });
}
