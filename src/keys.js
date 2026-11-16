// Injected into every page and into the browser UI. Turns key presses into combo
// strings like "Mod+Shift+T" and reports the ones bound to a browser action.
// ponytail: the bound-combo set is baked in when a tab is created; after a hotkey
// change, old tabs pick it up via a pushed update that lasts until they navigate.
(() => {
  if (window.__wispCombo) return;
  const mac = navigator.platform.startsWith('Mac');
  window.__wispCombo = e => {
    if (['Meta', 'Control', 'Alt', 'Shift'].includes(e.key)) return null;
    const p = [];
    if (mac ? e.metaKey : e.ctrlKey) p.push('Mod');
    if (mac && e.ctrlKey) p.push('Ctrl');
    if (!mac && e.metaKey) p.push('Meta');
    if (e.altKey) p.push('Alt');
    if (e.shiftKey) p.push('Shift');
    const c = e.code || e.key;
    p.push(c.startsWith('Key') ? c.slice(3) : c.startsWith('Digit') ? c.slice(5) : c);
    return p.join('+');
  };
  window.__wispKeys = new Set(__KEYS__);
  addEventListener('keydown', e => {
    if (window.__wispRecording) return;
    const c = window.__wispCombo(e);
    if (c && window.__wispKeys.has(c)) {
      e.preventDefault();
      e.stopImmediatePropagation();
      window.ipc.postMessage(JSON.stringify({ cmd: 'key', combo: c }));
    }
  }, true);
})();
