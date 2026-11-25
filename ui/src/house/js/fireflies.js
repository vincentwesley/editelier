// Firefly canvas swarm. startFireflies(canvas, {count}) -> {pause, resume, stop}. No deps.
// Reads --firefly / --firefly-core; sheds fireflies when frames run slow; one static frame under reduced motion.
export function startFireflies(canvas, { count = 28 } = {}) {
  const ctx = canvas.getContext("2d");
  const mq = matchMedia("(prefers-reduced-motion: reduce)");
  let W = 0, H = 0, n = count, raf = 0, paused = false, stopped = false, last = 0, slow = 0;
  const rnd = (a, b) => a + Math.random() * (b - a);
  const flies = Array.from({ length: count }, () => ({ x: rnd(0, 1), y: rnd(0, 1), a: rnd(0, 6.28), ph: rnd(0, 6.28), r: rnd(0.6, 1.4) }));

  const colors = () => {
    const cs = getComputedStyle(document.documentElement);
    return [cs.getPropertyValue("--firefly").trim() || "#9dff6a", cs.getPropertyValue("--firefly-core").trim() || "#eaffb0"];
  };
  const resize = () => {
    const dpr = Math.min(2, devicePixelRatio || 1);
    W = canvas.clientWidth || innerWidth;
    H = canvas.clientHeight || innerHeight;
    canvas.width = Math.round(W * dpr);
    canvas.height = Math.round(H * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  };
  const draw = (t, still) => {
    const [glow, core] = colors();
    ctx.clearRect(0, 0, W, H);
    ctx.globalCompositeOperation = "lighter";
    for (let i = 0; i < n; i++) {
      const f = flies[i];
      const al = still ? 0.35 + 0.65 * ((i * 0.37) % 1) : Math.max(0, Math.sin(t * 1.5 + f.ph)) ** 2;
      if (al < 0.02) continue;
      const x = f.x * W, y = f.y * H, r = 22 * f.r;
      const g = ctx.createRadialGradient(x, y, 0, x, y, r);
      g.addColorStop(0, core);
      g.addColorStop(0.12, glow);
      g.addColorStop(1, "transparent");
      ctx.globalAlpha = al * 0.8;
      ctx.fillStyle = g;
      ctx.fillRect(x - r, y - r, r * 2, r * 2);
    }
    ctx.globalAlpha = 1;
  };
  const frame = (now) => {
    raf = requestAnimationFrame(frame);
    const dt = last ? Math.min(0.1, (now - last) / 1000) : 0;
    last = now;
    for (let i = 0; i < n; i++) {
      const f = flies[i];
      f.a += rnd(-0.3, 0.3);
      f.x = (f.x + (Math.cos(f.a) * 12 * dt) / W + 1) % 1;
      f.y = (f.y + (Math.sin(f.a) * 12 * dt) / H + 1) % 1;
    }
    const t0 = performance.now();
    draw(now / 1000, false);
    // Governor: 12 slow frames in a row (draw > 12ms) -> shed 20% of the swarm, never below 6.
    slow = performance.now() - t0 > 12 ? slow + 1 : 0;
    if (slow > 12 && n > 6) { n = Math.max(6, Math.floor(n * 0.8)); slow = 0; }
  };
  const run = () => {
    cancelAnimationFrame(raf);
    raf = 0;
    if (stopped) return;
    if (mq.matches) return draw(0, true); // reduced motion: one static frame, never animate
    if (!paused && !document.hidden) { last = 0; raf = requestAnimationFrame(frame); }
  };
  const onResize = () => { resize(); if (!raf) draw(last / 1000, true); };
  resize();
  addEventListener("resize", onResize);
  document.addEventListener("visibilitychange", run);
  mq.addEventListener("change", run);
  run();
  return {
    pause: () => { paused = true; run(); },
    resume: () => { paused = false; run(); },
    stop: () => {
      stopped = true;
      run();
      removeEventListener("resize", onResize);
      document.removeEventListener("visibilitychange", run);
      mq.removeEventListener("change", run);
      ctx.clearRect(0, 0, W, H);
    },
  };
}
