import { describe, expect, it } from 'vitest';

import { PanelStore } from './panel-store';

describe('PanelStore', () => {
  it('keeps concurrent requests independent and maps completion to one conversation panel', () => {
    const store = new PanelStore();
    const one = store.ensureRequest('request-one');
    const two = store.ensureRequest('request-two');
    store.hide(one.id);
    store.setCollapsed(two.id, true);
    const completed = store.completeRequest('request-one', 'conversation-one');
    expect(completed).toMatchObject({
      conversationId: 'conversation-one',
      hidden: true,
    });
    expect(store.snapshot().panels).toHaveLength(2);
    expect(
      store.snapshot().panels.find((panel) => panel.id === two.id),
    ).toMatchObject({
      collapsed: true,
      hidden: false,
    });
  });

  it('reopens an existing panel and brings it to front without cancelling it', () => {
    const store = new PanelStore();
    const first = store.ensureRequest('request-one');
    const second = store.ensureRequest('request-two');
    store.hide(first.id);
    const reopened = store.reopen(first.id);
    expect(reopened).toMatchObject({ id: first.id, hidden: false });
    expect(reopened.zIndex).toBeGreaterThan(second.zIndex);
  });

  it('deduplicates a completed conversation to one panel instance', () => {
    const store = new PanelStore();
    const first = store.ensureRequest('request-one');
    store.ensureRequest('request-two');
    store.completeRequest('request-one', 'conversation-one');
    store.completeRequest('request-two', 'conversation-one');
    expect(store.snapshot().panels).toHaveLength(1);
    expect(store.snapshot().panels[0].id).toBe(first.id);
  });
});
