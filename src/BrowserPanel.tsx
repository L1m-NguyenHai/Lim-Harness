import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import { ArrowLeft, ArrowRight, Globe, Loader2, Plus, RotateCw, X } from "lucide-react";
import { browser, native } from "./core";
import type { BrowserState } from "./core";
import "./browser.css";

type Props = {
  open: boolean;
  /** A floating window covers the chat area: the native page must hide or it would draw over it. */
  obscured: boolean;
  onOpen: () => void;
  onClose: () => void;
};

/**
 * Side browser. Pages are native webviews owned by the Rust side (browser.rs); this panel only
 * draws the tab strip and address bar and reports where the page area is so the active tab
 * can be placed exactly over `.browser-viewport`.
 */
export function BrowserPanel({ open, obscured, onOpen, onClose }: Props) {
  const [state, setState] = useState<BrowserState>({ tabs: [], active: null });
  const [address, setAddress] = useState("");
  const [editing, setEditing] = useState(false);
  const [error, setError] = useState("");
  const viewport = useRef<HTMLDivElement>(null);
  const onOpenRef = useRef(onOpen);
  onOpenRef.current = onOpen;
  const active = state.tabs.find((t) => t.id === state.active);

  useEffect(() => {
    if (!native) return;
    let stop: (() => void) | undefined;
    void browser.state().then(setState).catch(() => undefined);
    void browser.listen((next) => {
      setState({ tabs: next.tabs, active: next.active });
      if (next.reveal) onOpenRef.current();
    }).then((unlisten) => { stop = unlisten; });
    return () => stop?.();
  }, []);

  // A browser page that had the keyboard keeps it even when you click back into the app
  // (WebView2 behaviour with several webviews in one window): hand it back on every click.
  const hasTabs = state.tabs.length > 0;
  useEffect(() => {
    if (!native || !hasTabs) return;
    // Only when a page really holds the keyboard: moving focus while the app already has it
    // blurs the clicked control (it reset the two-step "Xoá?" button before the second click).
    const reclaim = () => { if (!document.hasFocus()) void browser.focusMain().catch(() => undefined); };
    document.addEventListener("pointerdown", reclaim, true);
    return () => document.removeEventListener("pointerdown", reclaim, true);
  }, [hasTabs]);

  useEffect(() => { if (!editing) setAddress(active?.url === "about:blank" ? "" : active?.url ?? ""); }, [active?.url, active?.id, editing]);

  // Keep the native page glued to the viewport box: size changes, window resizes and layout
  // shifts (sidebar collapse) are all caught by comparing the rectangle.
  const visible = open && !obscured && state.tabs.length > 0;
  useEffect(() => {
    if (!native) return;
    let last = "";
    function report() {
      const box = viewport.current?.getBoundingClientRect();
      const bounds = box ? { x: box.left, y: box.top, width: box.width, height: box.height } : { x: 0, y: 0, width: 0, height: 0 };
      const key = `${visible}:${Math.round(bounds.x)}:${Math.round(bounds.y)}:${Math.round(bounds.width)}:${Math.round(bounds.height)}`;
      if (key === last) return;
      last = key;
      void browser.setBounds(bounds, visible && bounds.width > 0).catch(() => undefined);
    }
    report();
    const observer = new ResizeObserver(report);
    if (viewport.current) observer.observe(viewport.current);
    window.addEventListener("resize", report);
    const timer = window.setInterval(report, 500);
    return () => { observer.disconnect(); window.removeEventListener("resize", report); window.clearInterval(timer); };
  }, [visible]);
  // Hide pages when the panel unmounts or closes.
  useEffect(() => () => { if (native) void browser.setBounds({ x: 0, y: 0, width: 0, height: 0 }, false).catch(() => undefined); }, []);

  async function run(action: () => Promise<void>) {
    setError("");
    try { await action(); } catch (e) { setError(String(e)); }
  }
  function submit(event: FormEvent) {
    event.preventDefault();
    const target = address.trim();
    if (!target) return;
    setEditing(false);
    void run(() => browser.open(target, !active));
  }

  if (!open) return null;
  return (
    <section className="browser-panel" aria-label="Trình duyệt">
      <div className="browser-tabs" role="tablist">
        {state.tabs.map((tab) => (
          <div key={tab.id} role="tab" aria-selected={tab.id === state.active} className={`browser-tab ${tab.id === state.active ? "active" : ""}`} title={`${tab.title}\n${tab.url}`}>
            <button className="browser-tab-label" onClick={() => void run(() => browser.select(tab.id))}>
              {tab.loading ? <Loader2 size={12} className="spin" /> : <Globe size={12} />}
              <span>{tab.title || tab.url}</span>
            </button>
            <button className="browser-tab-close" aria-label={`Đóng tab ${tab.title}`} onClick={() => void run(() => browser.close(tab.id))}><X size={12} /></button>
          </div>
        ))}
        <button className="icon-button browser-new" aria-label="Tab mới" title="Tab mới" onClick={() => void run(() => browser.open("about:blank", true))}><Plus size={14} /></button>
        <button className="icon-button browser-hide" aria-label="Đóng trình duyệt" title="Ẩn trình duyệt" onClick={onClose}><X size={15} /></button>
      </div>
      <form className="browser-bar" onSubmit={submit}>
        <button type="button" className="icon-button" aria-label="Quay lại" disabled={!active} onClick={() => void run(() => browser.nav("back"))}><ArrowLeft size={15} /></button>
        <button type="button" className="icon-button" aria-label="Tiến tới" disabled={!active} onClick={() => void run(() => browser.nav("forward"))}><ArrowRight size={15} /></button>
        <button type="button" className="icon-button" aria-label="Tải lại" disabled={!active} onClick={() => void run(() => browser.nav("reload"))}><RotateCw size={14} /></button>
        <input value={address} onFocus={() => setEditing(true)} onBlur={() => setEditing(false)} onChange={(e) => setAddress(e.target.value)} placeholder="Nhập địa chỉ hoặc từ khóa tìm kiếm" aria-label="Địa chỉ" spellCheck={false} />
      </form>
      {error && <div className="browser-error" role="alert">{error}</div>}
      <div className="browser-viewport" ref={viewport}>
        {!native ? <p>Trình duyệt chỉ chạy trong bản desktop.</p>
          : !state.tabs.length ? <p>Chưa có tab nào. Nhập địa chỉ ở trên, hoặc bật Agent và nhờ Lim mở trang, tra cứu giúp bạn.</p>
          : obscured ? <p>Trình duyệt tạm ẩn khi có cửa sổ đang mở.</p> : null}
      </div>
    </section>
  );
}
