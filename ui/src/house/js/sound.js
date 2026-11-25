// Optional UI sounds. Off by default, never autoplays. Usage:
//   import { enableSound, play } from "./sound.js";
//   enableSound(document.querySelector("#sound"));   // <button type="button" aria-pressed="false">Sound</button>
//   play("coin");                                     // click | ok | warn | coin; no-op while sound is off
// The AudioContext is created only after a user gesture (the toggle click, or on a later page load once the user has interacted).
// Synth presets via ZzFX (MIT, js/zzfx.js). For sampled sounds, Kenney Interface Sounds (CC0, https://kenney.nl/assets/interface-sounds)
// is the recommended source; no audio files are bundled in the kit.
import { zzfx, zzfxCtx } from "./zzfx.js";

const PRESETS = {
  click: [.5, , 900, , .004, .02, 1],
  ok: [.6, , 523, .01, .05, .15, 1, 1, , , 200, .06],
  warn: [.6, , 180, .01, .1, .2, 2, , -4, , , , , .1],
  coin: [, , 1675, , .06, .24, 1, 1.82, , , 837, .06],
};
let on = false;
let ctxMade = false;

const save = (v) => { try { localStorage.setItem("sound", v ? "on" : "off"); } catch (e) { /* storage blocked */ } };
const saved = () => { try { return localStorage.getItem("sound") === "on"; } catch (e) { return false; } };

export const soundEnabled = () => on;

export function play(name) {
  const p = PRESETS[name];
  if (!on || !p) return;
  if (!ctxMade && !navigator.userActivation?.hasBeenActive) return; // never create a context before a gesture
  ctxMade = true;
  const ctx = zzfxCtx();
  if (ctx.state === "suspended") ctx.resume();
  zzfx(...p);
}

export function enableSound(button) {
  if (!button || typeof AudioContext === "undefined") return;
  const sync = () => button.setAttribute("aria-pressed", String(on));
  on = saved();
  sync();
  button.addEventListener("click", () => {
    on = !on;
    save(on);
    sync();
    play("click"); // the click is the gesture that creates the context
  });
}
