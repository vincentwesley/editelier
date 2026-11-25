// Optional day/night accent shift (opt-in). Usage: import { initDayNight } from "./daynight.js"; initDayNight();
// Sets data-daypart="dawn|day|dusk|night" on <html> from the local hour; tokens.css shifts decorative accents in the dark theme only.
// Pass { hour } to force an hour (demos/tests). Returns the daypart.
export function initDayNight({ hour = new Date().getHours() } = {}) {
  const part = hour >= 5 && hour < 8 ? "dawn" : hour >= 8 && hour < 17 ? "day" : hour >= 17 && hour < 20 ? "dusk" : "night";
  document.documentElement.setAttribute("data-daypart", part);
  return part;
}
