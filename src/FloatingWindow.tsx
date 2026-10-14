import { useEffect, useRef, useState } from "react";
import type { PointerEvent, ReactNode } from "react";
import { Maximize2, Minimize2, Minus, MoreHorizontal, X } from "lucide-react";
import "./windows.css";

export type WindowMode = "closed" | "open" | "minimized";
type Bounds = { x: number; y: number; width: number; height: number };
type Props = {
  id: string; title: string; mode: WindowMode; zIndex: number;
  width?: number; height?: number; offset?: number; children: ReactNode;
  onFocus: () => void; onClose: () => void; onMinimize: () => void;
};
function clamp(bounds: Bounds): Bounds {
  const width = Math.min(Math.max(300, bounds.width), window.innerWidth - 16);
  const height = Math.min(Math.max(220, bounds.height), window.innerHeight - 60);
  return { width, height, x: Math.max(8, Math.min(bounds.x, window.innerWidth - width - 8)), y: Math.max(8, Math.min(bounds.y, window.innerHeight - height - 44)) };
}

export default function FloatingWindow({ id, title, mode, zIndex, width = 760, height = 640, offset = 0, children, onFocus, onClose, onMinimize }: Props) {
  const [bounds, setBounds] = useState<Bounds>(() => clamp({ x: window.innerWidth > 900 ? 230 + offset : 16, y: 56 + offset, width, height }));
  const [maximized, setMaximized] = useState(false);
  const element = useRef<HTMLElement>(null);
  const gesture = useRef<{ pointer: number; x: number; y: number; bounds: Bounds; kind: "move" | "resize" } | null>(null);

  useEffect(() => {
    function resize() { setBounds((current) => clamp(current)); }
    window.addEventListener("resize", resize);
    return () => window.removeEventListener("resize", resize);
  }, []);
  useEffect(() => { if (mode === "open") element.current?.focus({ preventScroll: true }); }, [mode]);

  function begin(event: PointerEvent<HTMLElement>, kind: "move" | "resize") {
    if (maximized || event.button !== 0 || (kind === "move" && (event.target as HTMLElement).closest("button,details"))) return;
    event.preventDefault();
    gesture.current = { pointer: event.pointerId, x: event.clientX, y: event.clientY, bounds, kind };
    event.currentTarget.setPointerCapture(event.pointerId);
  }
  function move(event: PointerEvent<HTMLElement>) {
    const start = gesture.current;
    if (!start || start.pointer !== event.pointerId) return;
    const dx = event.clientX - start.x, dy = event.clientY - start.y;
    setBounds(clamp(start.kind === "move" ? { ...start.bounds, x: start.bounds.x + dx, y: start.bounds.y + dy } : { ...start.bounds, width: start.bounds.width + dx, height: start.bounds.height + dy }));
  }
  function end() { gesture.current = null; }
  function close() {
    onClose();
    document.querySelector<HTMLButtonElement>(`[data-window-opener="${id}"]`)?.focus();
  }

  return <section ref={element} className={`floating-window ${maximized ? "is-maximized" : ""}`} hidden={mode !== "open"} role="dialog" aria-modal="false" aria-labelledby={`${id}-window-title`} tabIndex={-1}
    style={{ zIndex, ...(maximized ? {} : { left: bounds.x, top: bounds.y, width: bounds.width, height: bounds.height }) }} onPointerDownCapture={onFocus}
    onKeyDown={(event) => { if (event.key === "Escape" && !(event.target as HTMLElement).closest("input,textarea,select")) { event.stopPropagation(); close(); } }}>
    <header className="window-titlebar" tabIndex={0} aria-label={`${title}: kéo để di chuyển, Alt và phím mũi tên để di chuyển bằng bàn phím`} onPointerDown={(event) => begin(event, "move")} onPointerMove={move} onPointerUp={end} onPointerCancel={end}
      onDoubleClick={(event) => { if (!(event.target as HTMLElement).closest("button,details")) setMaximized((current) => !current); }}
      onKeyDown={(event) => { if (!event.altKey || maximized || !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) return; event.preventDefault(); setBounds((current) => clamp({ ...current, x: current.x + (event.key === "ArrowRight" ? 24 : event.key === "ArrowLeft" ? -24 : 0), y: current.y + (event.key === "ArrowDown" ? 24 : event.key === "ArrowUp" ? -24 : 0) })); }}>
      <h2 id={`${id}-window-title`}>{title}</h2>
      <div className="window-actions">
        <details className="window-placement"><summary aria-label={`Vị trí và kích thước ${title}`} title="Vị trí và kích thước"><MoreHorizontal size={16} /></summary><div>
          {(["left", "center", "right"] as const).map((position) => <button key={position} onClick={(event) => { setMaximized(false); setBounds((current) => clamp({ ...current, x: position === "left" ? 8 : position === "right" ? window.innerWidth - current.width - 8 : (window.innerWidth - current.width) / 2, y: 24 })); event.currentTarget.closest("details")?.removeAttribute("open"); }}>{position === "left" ? "Căn trái" : position === "right" ? "Căn phải" : "Căn giữa"}</button>)}
          <button onClick={() => { setMaximized(false); setBounds((current) => clamp({ ...current, width: current.width - 64, height: current.height - 64 })); }}>Nhỏ hơn</button>
          <button onClick={() => { setMaximized(false); setBounds((current) => clamp({ ...current, width: current.width + 64, height: current.height + 64 })); }}>Lớn hơn</button>
        </div></details>
        <button className="icon-button" onClick={onMinimize} aria-label={`Thu gọn ${title}`} title="Thu gọn"><Minus size={16} /></button>
        <button className="icon-button" onClick={() => setMaximized((current) => !current)} aria-label={`${maximized ? "Khôi phục" : "Phóng to"} ${title}`} title={maximized ? "Khôi phục" : "Phóng to"}>{maximized ? <Minimize2 size={14} /> : <Maximize2 size={14} />}</button>
        <button className="icon-button" onClick={close} aria-label={`Đóng ${title}`} title="Đóng"><X size={17} /></button>
      </div>
    </header>
    <div className={`window-body ${id === "cookbook" ? "window-body-cookbook" : ""}`}>{children}</div>
    {!maximized && <button className="window-resizer" aria-label={`Đổi kích thước ${title}`} title="Kéo để đổi kích thước; dùng phím mũi tên khi focus" onPointerDown={(event) => begin(event, "resize")} onPointerMove={move} onPointerUp={end} onPointerCancel={end}
      onKeyDown={(event) => { if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) return; event.preventDefault(); setBounds((current) => clamp({ ...current, width: current.width + (event.key === "ArrowRight" ? 24 : event.key === "ArrowLeft" ? -24 : 0), height: current.height + (event.key === "ArrowDown" ? 24 : event.key === "ArrowUp" ? -24 : 0) })); }} />}
  </section>;
}
