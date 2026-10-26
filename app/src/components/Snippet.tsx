/** Render a search snippet whose matches are wrapped in \u0001 … \u0002. */
export function Snippet({ text }: { text: string }) {
  const parts = text.split(/(\u0001[^\u0002]*\u0002)/g).filter(Boolean);
  return (
    <span className="snippet">
      {parts.map((p, i) => (p.startsWith("\u0001") ? <mark key={i}>{p.slice(1, -1)}</mark> : <span key={i}>{p}</span>))}
    </span>
  );
}
