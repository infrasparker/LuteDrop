import { test } from 'node:test';
import assert from 'node:assert/strict';
import { effectScope, nextTick, shallowRef } from 'vue';
import Sortable from 'sortablejs';
import { useListReorder } from '../src/composables/useListReorder.ts';

// Exercise Vue lifecycle, persisted reorder callbacks, and DOM ownership without
// launching a browser. Pointer handling remains Sortable's responsibility.
class List {
  isConnected = true;
  constructor(ids) {
    this.children = ids.map((id) => ({ dataset: { reorderId: id }, parentNode: this }));
  }
  insertBefore(item, next) {
    this.children.splice(this.children.indexOf(item), 1);
    this.children.splice(next ? this.children.indexOf(next) : this.children.length, 0, item);
  }
  ids() { return this.children.map((item) => item.dataset.reorderId); }
}

async function harness(run) {
  const original = Sortable.create;
  const instances = [];
  Sortable.create = (element, options) => {
    const instance = { element, options, disabled: false, destroyed: false,
      option(name, value) { assert.equal(this.destroyed, false); this[name] = value; },
      destroy() { this.destroyed = true; } };
    instances.push(instance);
    return instance;
  };
  const scope = effectScope();
  try { await run(scope, instances); }
  finally { scope.stop(); Sortable.create = original; }
}

test('drag restores Vue DOM before saving and releases the handle after a rejected save', async () => {
  await harness(async (scope, instances) => {
    const list = new List(['music', 'rain', 'birds']);
    const container = shallowRef(null);
    const calls = [];
    scope.run(() => useListReorder(container, {
      item: '.track-card', handle: '.track-drag-handle', context: () => 'forest',
      ids: () => ['music', 'rain', 'birds'],
      move: async (id, index) => { assert.deepEqual(list.ids(), ['music', 'rain', 'birds']); calls.push([id, index]); return false; },
    }));
    container.value = list;
    await nextTick();
    const instance = instances[0];
    assert.equal(instance.options.handle, '.track-drag-handle');
    const rain = list.children[1];
    instance.options.onStart({ item: rain });
    list.insertBefore(rain, list.children[0]);
    await instance.options.onEnd({ item: rain, from: list, oldDraggableIndex: 1, newDraggableIndex: 0 });
    assert.deepEqual(calls, [['rain', 0]]);
    assert.deepEqual(list.ids(), ['music', 'rain', 'birds']);
    assert.equal(instance.disabled, false);
  });
});

test('switching scenes or changing the list during a drag cannot reorder the wrong layout', async () => {
  await harness(async (scope, instances) => {
    const list = new List(['a', 'b']);
    const container = shallowRef(null);
    let context = 'forest'; let ids = ['a', 'b']; let calls = 0;
    scope.run(() => useListReorder(container, { item: '.scene-item', handle: '.scene-drag-handle',
      context: () => context, ids: () => ids, move: async () => { calls++; } }));
    container.value = list;
    await nextTick();
    const options = instances[0].options;
    const item = list.children[1];
    options.onStart({ item }); context = 'desert';
    await options.onEnd({ item, from: list, oldDraggableIndex: 1, newDraggableIndex: 0 });
    assert.equal(calls, 0);
    options.onStart({ item }); ids = ['a', 'b', 'new'];
    await options.onEnd({ item, from: list, oldDraggableIndex: 1, newDraggableIndex: 0 });
    assert.equal(calls, 0);
  });
});

test('leaving the page while a save is pending cleans up sorting safely', async () => {
  await harness(async (scope, instances) => {
    const list = new List(['a', 'b']);
    const container = shallowRef(null);
    let finish;
    const pending = new Promise((resolve) => { finish = resolve; });
    scope.run(() => useListReorder(container, { item: '.track-card', handle: '.track-drag-handle',
      context: () => 'all', ids: () => ['a', 'b'], move: () => pending }));
    container.value = list;
    await nextTick();
    const options = instances[0].options;
    const item = list.children[1];
    options.onStart({ item });
    const ended = options.onEnd({ item, from: list, oldDraggableIndex: 1, newDraggableIndex: 0 });
    scope.stop(); finish(); await ended;
    assert.equal(instances[0].destroyed, true);
  });
});
