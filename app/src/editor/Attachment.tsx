import { Node, mergeAttributes } from "@tiptap/core";
import { NodeViewWrapper, ReactNodeViewRenderer, type ReactNodeViewProps } from "@tiptap/react";
import { Download, ExternalLink, File, FileAudio, FileImage, FileText, FileVideo } from "lucide-react";
import { assetSrc, formatBytes } from "./assets";
import { openAttachment, saveAttachment } from "../platform";
import { reportError } from "../ui/overlay";

function iconFor(mime: string) {
  if (mime.startsWith("image/")) return <FileImage size={22} />;
  if (mime.startsWith("audio/")) return <FileAudio size={22} />;
  if (mime.startsWith("video/")) return <FileVideo size={22} />;
  if (mime.startsWith("text/") || mime === "application/pdf") return <FileText size={22} />;
  return <File size={22} />;
}

function hashOf(src: string) {
  return src.replace("anynote://asset/", "");
}

function AttachmentView({ node }: ReactNodeViewProps) {
  const { src, name, size, mime } = node.attrs as { src: string; name: string; size: number; mime: string };
  const type = mime || "application/octet-stream";
  const open = () => openAttachment(hashOf(src), name).catch(reportError);
  const saveAs = () => saveAttachment(hashOf(src), name).catch(reportError);
  return (
    <NodeViewWrapper className="attachment" data-drag-handle="">
      <div className="attachment-card" contentEditable={false}>
        <span className="attachment-icon">{iconFor(type)}</span>
        <span className="attachment-meta">
          <span className="attachment-name">{name || "Attachment"}</span>
          <span className="attachment-size">{[formatBytes(size), type].filter(Boolean).join(" · ")}</span>
        </span>
        <button title="Open with the default app" onClick={open}>
          <ExternalLink size={16} />
        </button>
        <button title="Save a copy…" onClick={() => void saveAs()}>
          <Download size={16} />
        </button>
      </div>
      {type.startsWith("audio/") && <audio controls src={assetSrc(src)} contentEditable={false} />}
      {type.startsWith("video/") && <video controls src={assetSrc(src)} contentEditable={false} />}
    </NodeViewWrapper>
  );
}

/** An embedded file stored in the asset store. */
export const Attachment = Node.create({
  name: "attachment",
  group: "block",
  atom: true,
  draggable: true,
  selectable: true,

  addAttributes() {
    return {
      src: { default: null },
      name: { default: "" },
      size: { default: 0 },
      mime: { default: "" },
    };
  },

  parseHTML() {
    return [
      {
        tag: "div[data-attachment]",
        getAttrs: (el) => ({
          src: el.getAttribute("data-src"),
          name: el.getAttribute("data-name") ?? "",
          size: Number(el.getAttribute("data-size")) || 0,
          mime: el.getAttribute("data-mime") ?? "",
        }),
      },
    ];
  },

  renderHTML({ node, HTMLAttributes }) {
    return [
      "div",
      mergeAttributes(HTMLAttributes, {
        "data-attachment": "",
        "data-src": node.attrs.src,
        "data-name": node.attrs.name,
        "data-size": node.attrs.size,
        "data-mime": node.attrs.mime,
      }),
      node.attrs.name,
    ];
  },

  renderText({ node }) {
    return `📎 ${node.attrs.name}`;
  },

  addNodeView() {
    return ReactNodeViewRenderer(AttachmentView, {
      stopEvent: ({ event }) => !!(event.target as HTMLElement | null)?.closest?.("button, audio, video"),
    });
  },
});
