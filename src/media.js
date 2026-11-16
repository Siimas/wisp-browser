// Injected into every page (main frame only). Reports what the page is playing so the
// sidebar can show a player, and carries out the player's buttons. Event-driven: no
// timer runs unless something has just started playing. Everything sent here is
// treated by the app as hostile (see media.rs); this script is just the honest case.
(() => {
  if (window.__wispMedia) return;
  const send = m => window.ipc.postMessage(JSON.stringify(m));
  const session = navigator.mediaSession, handlers = {};
  let cur = null, meta = null, shown = false, due = 0;
  const report = () => {
    if (!cur) return;
    const d = cur.duration;
    send({ cmd: 'media', playing: !cur.paused && !cur.ended, muted: cur.muted,
      title: String((meta && meta.title) || document.title).slice(0, 200), artist: String((meta && meta.artist) || '').slice(0, 200),
      pos: cur.currentTime || 0, dur: isFinite(d) ? d : 0, rate: cur.playbackRate,
      next: !!handlers.nexttrack, prev: !!handlers.previoustrack });
  };
  const onEvent = e => {
    const m = e.target;
    if (!(m instanceof HTMLMediaElement)) return;
    if (e.type === 'play' || e.type === 'playing') cur = m;
    if (m !== cur) return;
    if (e.type === 'ended' || e.type === 'emptied') { cur = null; if (shown) { shown = false; send({ cmd: 'media', gone: true }); } return; }
    if (shown) return report();
    // Like a short ping, a sound that stops within a second is not worth a player.
    if (e.type === 'playing' && !due) due = setTimeout(() => { due = 0; if (cur && !cur.paused) { shown = true; report(); } }, 1000);
  };
  const EVENTS = ['play', 'playing', 'pause', 'ended', 'emptied', 'seeked', 'ratechange', 'volumechange', 'durationchange'];
  for (const t of EVENTS) addEventListener(t, onEvent, true); // media events do not bubble, but they are captured
  // A sound made with `new Audio()` is never in the document: follow it once it plays.
  const watched = new WeakSet(), play = HTMLMediaElement.prototype.play;
  HTMLMediaElement.prototype.play = function () {
    if (!this.isConnected && !watched.has(this)) { watched.add(this); for (const t of EVENTS) this.addEventListener(t, onEvent); }
    return play.apply(this, arguments);
  };
  if (session) { // what the page says it is playing, and which buttons it handles
    const proto = Object.getPrototypeOf(session), desc = Object.getOwnPropertyDescriptor(proto, 'metadata'), set = proto.setActionHandler;
    if (desc) Object.defineProperty(session, 'metadata', { configurable: true, get: () => desc.get.call(session),
      set: v => { desc.set.call(session, v); meta = v; if (shown) report(); } });
    session.setActionHandler = function (name, fn) { handlers[name] = fn; set.call(this, name, fn); if (shown) report(); };
  }
  // The player's buttons. Only these, called by the app with a fixed name and a number.
  window.__wispMedia = (op, at) => {
    if (!cur) return;
    const h = handlers[{ play: 'play', pause: 'pause', next: 'nexttrack', prev: 'previoustrack', seek: 'seekto' }[op]];
    if (op === 'mute') {
      const on = !cur.muted;
      for (const m of [cur, ...document.querySelectorAll('audio, video')]) m.muted = on;
    } else if (h) h(op === 'seek' ? { action: 'seekto', seekTime: at } : { action: op });
    else if (op === 'play') cur.play();
    else if (op === 'pause') cur.pause();
    else if (op === 'prev') cur.currentTime = 0;
    else if (op === 'seek') cur.currentTime = Math.min(at, cur.duration || 0);
  };
})();
