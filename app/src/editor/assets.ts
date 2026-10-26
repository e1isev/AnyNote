import type { Editor } from "@tiptap/core";
import { api } from "../api";
import { assetUrl } from "../platform";

const PREFIX = "anynote://asset/";

/** Map a stored `anynote://asset/<hash>` URL to one the webview can load. */
export function assetSrc(url: string | null | undefined): string {
  if (!url) return "";
  if (!url.startsWith(PREFIX)) return url;
  return assetUrl(url.slice(PREFIX.length));
}

/** Inverse of {@link assetSrc}, for HTML pasted from our own pages. */
export function canonicalAssetUrl(src: string | null): string | null {
  if (!src) return src;
  const m = /^(?:anynote:\/\/localhost|http:\/\/anynote\.localhost|https?:\/\/[^/]+)?\/asset\/([0-9a-f]{64})$/.exec(src);
  return m ? PREFIX + m[1] : src;
}

export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return "";
  const units = ["B", "KB", "MB", "GB"];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return `${n.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

/** Store files and insert them: images inline, anything else as attachments. */
export async function insertFiles(editor: Editor, files: File[], pos?: number) {
  for (const file of files) {
    const bytes = new Uint8Array(await file.arrayBuffer());
    const info = await api.storeAsset(bytes, file.name, file.type);
    const node = info.mime.startsWith("image/") && !/emf|wmf|tiff/.test(info.mime)
      ? { type: "image", attrs: { src: info.url, alt: file.name.replace(/\.[^.]+$/, "") } }
      : { type: "attachment", attrs: { src: info.url, name: file.name, size: info.size, mime: info.mime } };
    const chain = editor.chain().focus();
    if (pos !== undefined) chain.insertContentAt(pos, node);
    else chain.insertContent(node);
    chain.run();
  }
}

/** Let the user pick files and insert them. */
export function pickFiles(editor: Editor, accept?: string) {
  const input = document.createElement("input");
  input.type = "file";
  input.multiple = true;
  if (accept) input.accept = accept;
  input.onchange = () => {
    if (input.files?.length) void insertFiles(editor, Array.from(input.files));
  };
  input.click();
}
