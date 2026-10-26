import { useEffect, useMemo, useState } from "react";
import qrcode from "qrcode-generator";
import { Smartphone } from "lucide-react";
import { call } from "../platform";
import { reportError } from "../ui/overlay";

interface PhoneStatus {
  enabled: boolean;
  urls: string[];
}

/** A QR code drawn as SVG squares (no HTML injection, crisp at any size). */
function QrCode({ text, size = 220 }: { text: string; size?: number }) {
  const cells = useMemo(() => {
    const qr = qrcode(0, "M");
    qr.addData(text);
    qr.make();
    const n = qr.getModuleCount();
    const rects: string[] = [];
    for (let r = 0; r < n; r++) for (let c = 0; c < n; c++) if (qr.isDark(r, c)) rects.push(`M${c} ${r}h1v1h-1z`);
    return { n, path: rects.join("") };
  }, [text]);
  return (
    <svg className="qr" width={size} height={size} viewBox={`-2 -2 ${cells.n + 4} ${cells.n + 4}`} role="img" aria-label="QR code">
      <rect x={-2} y={-2} width={cells.n + 4} height={cells.n + 4} fill="#fff" />
      <path d={cells.path} fill="#000" />
    </svg>
  );
}

/** Let a phone or tablet on the same network open AnyNote (browser version). */
export function PhoneDialog({ onClose }: { onClose: () => void }) {
  const [status, setStatus] = useState<PhoneStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    call<PhoneStatus>("phone_access", { enable: true })
      .then(setStatus)
      .catch((e) => setError(String(e instanceof Error ? e.message : e)));
  }, []);

  const turnOff = () =>
    call<PhoneStatus>("phone_access", { enable: false })
      .then(onClose)
      .catch(reportError);

  const url = status?.urls[0];
  return (
    <div className="modal-backdrop" onMouseDown={onClose}>
      <div className="modal phone-dialog" role="dialog" aria-label="Use on phone or tablet" onMouseDown={(e) => e.stopPropagation()}>
        <h2>
          <Smartphone size={18} /> Use AnyNote on your phone or tablet
        </h2>
        {error && <p className="error-text">{error}</p>}
        {!status && !error && <p>Starting…</p>}
        {status && !url && (
          <p>
            This computer isn't connected to a network yet. Connect it to Wi-Fi, or turn on your phone's hotspot and connect
            this computer to it (no internet needed), then open this again.
          </p>
        )}
        {url && (
          <>
            <ol className="phone-steps">
              <li>Put your phone on the same Wi-Fi as this computer. No internet is needed: your phone's hotspot works too.</li>
              <li>Scan this code with the phone's camera, or type the address into its browser.</li>
            </ol>
            <div className="qr-wrap">
              <QrCode text={url} />
            </div>
            <p className="phone-url">{url}</p>
            {status.urls.length > 1 && (
              <details>
                <summary>Other addresses of this computer</summary>
                {status.urls.slice(1).map((u) => (
                  <p key={u} className="phone-url small">
                    {u}
                  </p>
                ))}
              </details>
            )}
            <p className="muted small">
              Your notes stay on this computer; the phone works on them over the network. Anyone on your network with this
              link can open your notes, so keep it to yourself.
            </p>
          </>
        )}
        <div className="modal-actions">
          {status?.enabled && (
            <button className="danger" onClick={() => void turnOff()}>
              Turn off phone access
            </button>
          )}
          <button className="primary" onClick={onClose}>
            Done
          </button>
        </div>
      </div>
    </div>
  );
}
