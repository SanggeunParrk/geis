// Tweens: a value moving from where it is to where it should be, over time.
//
// The map animates by interpolation rather than by scripted motion: every animated quantity
// - the camera, a pulse's phase, a hover's glow - is a `Tween` that knows its start, its end
// and its easing, and the frame loop asks each for its value now. Pure apart from the clock
// it is given, so the maths is testable.

export type Easing = (t: number) => number;

export const easeOutCubic: Easing = (t) => 1 - Math.pow(1 - t, 3);
export const easeInOutCubic: Easing = (t) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);
export const linear: Easing = (t) => t;

export class Tween {
  private from: number;
  private to: number;
  private startedAt: number;
  private duration: number;
  private easing: Easing;

  constructor(value: number) {
    this.from = value;
    this.to = value;
    this.startedAt = 0;
    this.duration = 0;
    this.easing = linear;
  }

  /** Heads for `to` from wherever the value is now. */
  go(to: number, now: number, duration: number, easing: Easing = easeOutCubic): void {
    this.from = this.at(now);
    this.to = to;
    this.startedAt = now;
    this.duration = duration;
    this.easing = easing;
  }

  /** Jumps, no motion. */
  set(value: number): void {
    this.from = value;
    this.to = value;
    this.duration = 0;
  }

  at(now: number): number {
    if (this.duration <= 0) return this.to;
    const t = Math.min(1, Math.max(0, (now - this.startedAt) / this.duration));
    return this.from + (this.to - this.from) * this.easing(t);
  }

  target(): number {
    return this.to;
  }

  done(now: number): boolean {
    return this.duration <= 0 || now >= this.startedAt + this.duration;
  }
}

/** A one-shot pulse: 0 → 1 over `duration`, then stays at 1 - read as "how far along". */
export class Pulse {
  private startedAt = -Infinity;
  private duration = 1;

  fire(now: number, duration: number): void {
    this.startedAt = now;
    this.duration = duration;
  }

  /** 0 at the start, 1 when over; over when nothing was ever fired. */
  phase(now: number): number {
    // A pulse of no length is over at once. As a divisor, zero made the phase NaN at the
    // moment it fired - neither started nor over, and NaN in whatever it was drawn with.
    if (!(this.duration > 0)) return 1;
    return Math.min(1, Math.max(0, (now - this.startedAt) / this.duration));
  }

  active(now: number): boolean {
    return this.phase(now) < 1;
  }
}
