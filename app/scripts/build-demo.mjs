// Pack the demo build (dist-demo/) into one self-contained HTML file:
// scripts and styles inline, fonts as data: URIs. Used for the online demo,
// whose host only serves a single page.
//   VITE_DEMO=1 npx vite build --outDir dist-demo && node scripts/build-demo.mjs
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const dist = join(root, "dist-demo");
const html = readFileSync(join(dist, "index.html"), "utf8");
const MIME = { woff2: "font/woff2", woff: "font/woff", ttf: "font/ttf", svg: "image/svg+xml", png: "image/png" };

const jsPath = /<script[^>]*src="\/?([^"]+\.js)"[^>]*><\/script>/.exec(html)[1];
const cssPaths = [...html.matchAll(/<link[^>]*rel="stylesheet"[^>]*href="\/?([^"]+\.css)"[^>]*>/g)].map((m) => m[1]);

let css = cssPaths.map((p) => readFileSync(join(dist, p), "utf8")).join("\n");
css = css.replace(/url\(\/?(assets\/[^)"']+)\)/g, (_, file) => {
  const ext = file.split(".").pop();
  const data = readFileSync(join(dist, file)).toString("base64");
  return `url(data:${MIME[ext] ?? "application/octet-stream"};base64,${data})`;
});
// "</script" inside the bundle would end the inline script early.
const js = readFileSync(join(dist, jsPath), "utf8")
  .replace(/<\/script/gi, "<\\/script")
  // Literal U+FFFD in library strings: the same character, written as an escape.
  .replace(/\uFFFD/g, "\\uFFFD");

const page = `<title>AnyNote Demo</title>
<style>
${css}
</style>
<div id="root"><p id="boot-message" style="font-family: sans-serif; padding: 2em">Loading AnyNote…</p></div>
<script type="module">
${js}
</script>
`;
mkdirSync(join(root, "demo-page"), { recursive: true });
writeFileSync(join(root, "demo-page", "anynote-demo.html"), page);
console.log(`demo-page/anynote-demo.html: ${(page.length / 1024 / 1024).toFixed(2)} MB`);
