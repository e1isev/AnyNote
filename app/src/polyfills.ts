// Small polyfills for older browsers (e.g. Chromium 87–92 on Raspbian 11),
// covering APIs used by the app and its editor libraries.
/* eslint-disable @typescript-eslint/no-explicit-any */

function at(this: any, n: number) {
  n = Math.trunc(n) || 0;
  if (n < 0) n += this.length;
  return n < 0 || n >= this.length ? undefined : this[n];
}

for (const proto of [Array.prototype, String.prototype] as any[]) {
  if (!proto.at) Object.defineProperty(proto, "at", { value: at, writable: true, configurable: true });
}

if (!(Object as any).hasOwn) {
  Object.defineProperty(Object, "hasOwn", {
    value: (o: object, k: PropertyKey) => Object.prototype.hasOwnProperty.call(o, k),
    writable: true,
    configurable: true,
  });
}

if (!(Array.prototype as any).findLast) {
  Object.defineProperty(Array.prototype, "findLast", {
    value(this: any[], fn: (v: any, i: number, a: any[]) => boolean) {
      for (let i = this.length - 1; i >= 0; i--) if (fn(this[i], i, this)) return this[i];
      return undefined;
    },
    writable: true,
    configurable: true,
  });
  Object.defineProperty(Array.prototype, "findLastIndex", {
    value(this: any[], fn: (v: any, i: number, a: any[]) => boolean) {
      for (let i = this.length - 1; i >= 0; i--) if (fn(this[i], i, this)) return i;
      return -1;
    },
    writable: true,
    configurable: true,
  });
}

if (typeof (globalThis as any).structuredClone !== "function") {
  (globalThis as any).structuredClone = (v: unknown) => (v === undefined ? v : JSON.parse(JSON.stringify(v)));
}
