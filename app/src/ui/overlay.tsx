import { useEffect, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { createPortal } from "react-dom";

// Promise-based dialogs, toasts and context menus rendered by <OverlayHost/>.

type Dialog =
  | {
      kind: "prompt";
      title: string;
      label?: string;
      initial: string;
      placeholder?: string;
      multiline?: boolean;
      monospace?: boolean;
      confirmText?: string;
      resolve: (v: string | null) => void;
    }
  | { kind: "confirm"; title: string; message: string; confirmText: string; danger?: boolean; resolve: (v: boolean) => void };

interface Toast {
  id: number;
  message: string;
  kind: "info" | "error" | "success";
}

export interface MenuItem {
  label: string;
  icon?: ReactNode;
  danger?: boolean;
  disabled?: boolean;
  onSelect: () => void;
}
export type MenuEntry = MenuItem | "separator" | { colors: string[]; current: string | null; onPick: (c: string | null) => void };

interface Menu {
  x: number;
  y: number;
  items: MenuEntry[];
}

let dialog: Dialog | null = null;
let dialogId = 0;
let toasts: Toast[] = [];
let menu: Menu | null = null;
let version = 0;
const listeners = new Set<() => void>();
const emit = () => {
  version++;
  listeners.forEach((l) => l());
};

export function promptText(opts: {
  title: string;
  label?: string;
  initial?: string;
  placeholder?: string;
  multiline?: boolean;
  monospace?: boolean;
  confirmText?: string;
}): Promise<string | null> {
  return new Promise((resolve) => {
    dialog?.resolve(null as never);
    dialogId++;
    dialog = { kind: "prompt", initial: "", ...opts, resolve };
    emit();
  });
}

export function confirmAction(opts: { title: string; message: string; confirmText?: string; danger?: boolean }): Promise<boolean> {
  return new Promise((resolve) => {
    dialog?.resolve(null as never);
    dialogId++;
    dialog = { kind: "confirm", confirmText: "OK", ...opts, resolve };
    emit();
  });
}

let toastId = 0;
export function toast(message: string, kind: Toast["kind"] = "info") {
  const id = ++toastId;
  toasts = [...toasts, { id, message, kind }];
  emit();
  setTimeout(() => {
    toasts = toasts.filter((t) => t.id !== id);
    emit();
  }, kind === "error" ? 8000 : 4000);
}

export function reportError(e: unknown) {
  console.error(e);
  toast(e instanceof Error ? e.message : String(e), "error");
}

export function openMenu(event: { clientX: number; clientY: number; preventDefault?: () => void }, items: MenuEntry[]) {
  event.preventDefault?.();
  menu = { x: event.clientX, y: event.clientY, items };
  emit();
}

function closeDialog() {
  dialog = null;
  emit();
}

function closeMenu() {
  menu = null;
  emit();
}

function PromptDialog({ d }: { d: Extract<Dialog, { kind: "prompt" }> }) {
  const [value, setValue] = useState(d.initial);
  const ref = useRef<HTMLInputElement & HTMLTextAreaElement>(null);
  useEffect(() => {
    ref.current?.focus();
    ref.current?.select();
  }, []);
  const submit = () => {
    d.resolve(value);
    closeDialog();
  };
  const cancel = () => {
    d.resolve(null);
    closeDialog();
  };
  const common = {
    ref,
    value,
    placeholder: d.placeholder,
    className: d.monospace ? "monospace" : undefined,
    onChange: (e: React.ChangeEvent<HTMLInputElement & HTMLTextAreaElement>) => setValue(e.target.value),
    onKeyDown: (e: React.KeyboardEvent) => {
      if (e.key === "Escape") cancel();
      if (e.key === "Enter" && (!d.multiline || e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        submit();
      }
    },
  };
  return (
    <div className="modal-backdrop" onMouseDown={cancel}>
      <div className="modal" role="dialog" aria-label={d.title} onMouseDown={(e) => e.stopPropagation()}>
        <h2>{d.title}</h2>
        {d.label && <label>{d.label}</label>}
        {d.multiline ? <textarea rows={5} {...common} /> : <input {...common} />}
        <div className="modal-actions">
          <button onClick={cancel}>Cancel</button>
          <button className="primary" onClick={submit}>
            {d.confirmText ?? "OK"}
          </button>
        </div>
      </div>
    </div>
  );
}

function ConfirmDialog({ d }: { d: Extract<Dialog, { kind: "confirm" }> }) {
  const done = (v: boolean) => {
    d.resolve(v);
    closeDialog();
  };
  return (
    <div className="modal-backdrop" onMouseDown={() => done(false)}>
      <div
        className="modal"
        role="alertdialog"
        aria-label={d.title}
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.key === "Escape" && done(false)}
      >
        <h2>{d.title}</h2>
        <p>{d.message}</p>
        <div className="modal-actions">
          <button onClick={() => done(false)}>Cancel</button>
          <button className={d.danger ? "danger" : "primary"} autoFocus onClick={() => done(true)}>
            {d.confirmText}
          </button>
        </div>
      </div>
    </div>
  );
}

function ContextMenu({ m }: { m: Menu }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x: m.x, y: m.y });
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({ x: Math.min(m.x, window.innerWidth - r.width - 8), y: Math.min(m.y, window.innerHeight - r.height - 8) });
    const close = (e: Event) => {
      if (!el.contains(e.target as Node)) closeMenu();
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && closeMenu();
    window.addEventListener("mousedown", close, true);
    window.addEventListener("keydown", esc);
    window.addEventListener("blur", closeMenu);
    return () => {
      window.removeEventListener("mousedown", close, true);
      window.removeEventListener("keydown", esc);
      window.removeEventListener("blur", closeMenu);
    };
  }, [m]);
  return (
    <div ref={ref} className="context-menu" style={{ left: pos.x, top: pos.y }} role="menu">
      {m.items.map((item, i) => {
        if (item === "separator") return <div key={i} className="menu-separator" />;
        if ("colors" in item)
          return (
            <div key={i} className="menu-colors">
              <button className={item.current ? "swatch none" : "swatch none active"} title="No colour" onClick={() => (item.onPick(null), closeMenu())} />
              {item.colors.map((c) => (
                <button
                  key={c}
                  className={c === item.current ? "swatch active" : "swatch"}
                  style={{ background: c }}
                  title={c}
                  onClick={() => (item.onPick(c), closeMenu())}
                />
              ))}
            </div>
          );
        return (
          <button
            key={i}
            role="menuitem"
            disabled={item.disabled}
            className={item.danger ? "menu-item danger" : "menu-item"}
            onClick={() => {
              closeMenu();
              item.onSelect();
            }}
          >
            <span className="menu-icon">{item.icon}</span>
            {item.label}
          </button>
        );
      })}
    </div>
  );
}

export function OverlayHost() {
  useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => version,
  );
  return createPortal(
    <>
      {dialog?.kind === "prompt" && <PromptDialog key={dialogId} d={dialog} />}
      {dialog?.kind === "confirm" && <ConfirmDialog d={dialog} />}
      {menu && <ContextMenu m={menu} />}
      <div className="toasts" aria-live="polite">
        {toasts.map((t) => (
          <div key={t.id} className={`toast ${t.kind}`}>
            {t.message}
          </div>
        ))}
      </div>
    </>,
    document.body,
  );
}
