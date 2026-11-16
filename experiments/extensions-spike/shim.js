/* Wisp spike shim: stand-ins for Chrome APIs WebKit lacks. Test copy only. */
(() => { const g = globalThis, targets = [g.chrome, g.browser].filter(Boolean);
  const ev = () => ({ addListener() {}, removeListener() {}, hasListener() { return false; } });
  for (const c of targets) {
    if (!c.idle) c.idle = { setDetectionInterval() {}, onStateChanged: ev(), queryState: async () => "active" };
    if (!c.runtime.ContextType) c.runtime.ContextType = { OFFSCREEN_DOCUMENT: "OFFSCREEN_DOCUMENT", POPUP: "POPUP", BACKGROUND: "BACKGROUND", TAB: "TAB", SIDE_PANEL: "SIDE_PANEL" };
    if (!c.runtime.getContexts) c.runtime.getContexts = async () => [];
  }
})();
