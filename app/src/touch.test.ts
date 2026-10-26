import { describe, expect, it } from "vitest";
import { isLongSwipe } from "./touch";

describe("isLongSwipe", () => {
  it("needs a long, mostly sideways, quick movement", () => {
    expect(isLongSwipe(300, 20, 300, 800)).toBe(true); // 37% of the width
    expect(isLongSwipe(-300, 40, 500, 800)).toBe(true); // to the left
    expect(isLongSwipe(150, 10, 300, 800)).toBe(false); // too short on a wide screen
    expect(isLongSwipe(130, 10, 300, 390)).toBe(true); // a third of a phone screen
    expect(isLongSwipe(300, 200, 300, 800)).toBe(false); // too diagonal: that's scrolling
    expect(isLongSwipe(300, 20, 1500, 800)).toBe(false); // too slow: a drag
  });
});
