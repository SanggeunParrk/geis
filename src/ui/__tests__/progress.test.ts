import { describe, expect, it } from "vitest";
import { percent } from "../progress";

describe("the gauge's figure", () => {
  it("is a whole percentage, kept between 0 and 100", () => {
    expect(percent(0.256)).toBe(26);
    expect(percent(-1)).toBe(0);
    expect(percent(3)).toBe(100);
  });

  it("is nothing when there is no telling", () => {
    expect(percent(null)).toBeNull();
    expect(percent(Number.NaN)).toBeNull();
  });
});
