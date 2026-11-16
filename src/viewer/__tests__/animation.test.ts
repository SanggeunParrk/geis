import { afterEach, describe, expect, it, vi } from 'vitest';
import { Viewer } from '../viewer';

// The frame scheduler can be exercised without a GPU: drawing records the pose it saw.
function harness() {
  let queued: FrameRequestCallback[] = [];
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
    queued.push(callback);
    return queued.length;
  });
  const poses: number[] = [];
  const viewer = Object.assign(Object.create(Viewer.prototype), {
    dirty: false, running: false, disposed: false, animation: null,
    rowState: { pitch: 0, focus: 0, shrink: 1, turns: [0] },
    draw() { poses.push(this.rowState.turns[0]); },
  }) as Viewer;
  return {
    viewer, poses,
    pending: () => queued.length,
    frame(time: number) {
      const callbacks = queued;
      queued = [];
      callbacks.forEach(callback => callback(time));
    },
  };
}

afterEach(() => vi.unstubAllGlobals());

describe('animated viewer frames', () => {
  it('draws the current pose in the same display callback, with no frame of lag', () => {
    const h = harness();
    h.viewer.animate(time => {
      h.viewer.setRow(21, 0, 0.62, [time / 1000]);
      return true;
    });
    h.frame(100);
    expect(h.poses).toEqual([0.1]);
    expect(h.pending()).toBe(1);
    h.frame(116.7);
    expect(h.poses).toEqual([0.1, 0.1167]);
    expect(h.pending()).toBe(1);
  });

  it('coalesces setters and replacement animations into one render', () => {
    const h = harness();
    h.viewer.animate(() => { throw new Error('obsolete animation'); });
    h.viewer.animate(() => {
      h.viewer.setRow(21, 0, 0.62, [1]);
      h.viewer.setRow(21, 0, 0.62, [2]);
      return true;
    });
    h.viewer.requestFrame();
    expect(h.pending()).toBe(1);
    h.frame(20);
    expect(h.poses).toEqual([2]);
    expect(h.pending()).toBe(1);
  });

  it('stops scheduling when its screen detaches and can resume on return', () => {
    const h = harness();
    h.viewer.animate(() => false);
    h.frame(20);
    expect(h.pending()).toBe(0);
    h.viewer.animate(() => {
      h.viewer.setRow(21, 0, 0.62, [3]);
      return true;
    });
    h.frame(10000);
    expect(h.poses.at(-1)).toBe(3);
    expect(h.pending()).toBe(1);
  });

  it('keeps an unchanged, non-animated viewer idle', () => {
    const h = harness();
    h.viewer.requestFrame();
    h.frame(20);
    expect(h.poses).toHaveLength(1);
    expect(h.pending()).toBe(0);
  });
});
