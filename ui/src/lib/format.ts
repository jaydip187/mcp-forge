/** A duration in milliseconds as `123 ms`, `1.23 s` or `12.3 s`; `–` when unknown. */
export function ms(n: number | null): string {
  if (n === null) return "–";
  return n < 1000 ? `${n} ms` : `${(n / 1000).toFixed(n < 10_000 ? 2 : 1)} s`;
}
