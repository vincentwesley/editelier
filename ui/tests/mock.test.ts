import { describe, expect, it, vi } from "vitest";
import { createMock } from "../src/api/mock";
import { TICKS_PER_SECOND as S, type CommandMap, type CommandName, type Snapshot } from "../src/api/types";
import { clipLength, frameTicks, mainStarts } from "../src/model";

async function setup() {
  const b = createMock({ tick: 1 });
  const run = <K extends CommandName>(name: K, args: CommandMap[K]) => b.invoke<Snapshot>("run_command", { name, args });
  await b.invoke("project_new", { name: "Test", aspect: "16:9", export_preset: "youtube-1080", keymap: "editelier" });
  const snap = await b.invoke<Snapshot>("import_media", { paths: ["a.mp4", "b.mp4", "song.mp3"] });
  const [a, bb, song] = snap.project.media;
  return { b, run, a: a!, bb: bb!, song: song! };
}

describe("mock backend", () => {
  it("imports with probe metadata and flags VFR / 4K", async () => {
    const b = createMock({ tick: 1 });
    await b.invoke("project_new", { name: "P", aspect: "9:16" });
    const snap = await b.invoke<Snapshot>("import_media", { paths: ["C:\\clips\\PXL_phone.mp4", "drone 4K.mov"] });
    const [phone, drone] = snap.project.media;
    expect(snap.project.canvas).toMatchObject({ width: 1080, height: 1920 });
    expect(phone).toMatchObject({ name: "PXL_phone.mp4", kind: "video", probe: { is_vfr: true } });
    expect(drone!.probe.width).toBe(3840);
    expect(drone!.proxy.status).toBe("building");
  });

  it("append, split, undo, redo round-trip with labels", async () => {
    const { run, a, bb } = await setup();
    await run("timeline.append", { media_id: a.id });
    let snap = await run("timeline.append", { media_id: bb.id });
    expect(snap.project.timeline.main).toHaveLength(2);
    const total = snap.project.timeline.main.reduce((n, c) => n + clipLength(c), 0);

    const at = 63 * frameTicks(snap.project.canvas.frame_rate);
    snap = await run("clip.split", { at });
    expect(snap.project.timeline.main).toHaveLength(3);
    expect(snap.undo_label).toBe("Split clip");
    expect(mainStarts(snap.project.timeline.main)[1]).toBe(at);
    expect(snap.project.timeline.main.reduce((n, c) => n + clipLength(c), 0)).toBe(total);

    snap = await run("history.undo", {});
    expect(snap.project.timeline.main).toHaveLength(2);
    expect(snap.redo_label).toBe("Split clip");
    snap = await run("history.redo", {});
    expect(snap.project.timeline.main).toHaveLength(3);
    expect(snap.undo_label).toBe("Split clip");
  });

  it("ripple delete closes the gap; move reorders; trims clamp to the source", async () => {
    const { run, a, bb } = await setup();
    await run("timeline.append", { media_id: a.id });
    let snap = await run("timeline.append", { media_id: bb.id });
    const [c1, c2] = snap.project.timeline.main;
    snap = await run("clip.move", { clip_id: c2!.id, index: 0 });
    expect(snap.project.timeline.main.map((c) => c.id)).toEqual([c2!.id, c1!.id]);
    snap = await run("clip.trim", { clip_id: c1!.id, edge: "out", delta: 999 * S });
    expect(snap.project.timeline.main[1]!.source_out).toBe(c1!.source_out);
    const f = frameTicks(snap.project.canvas.frame_rate);
    snap = await run("clip.trim", { clip_id: c1!.id, edge: "in", delta: 30 * f });
    expect(snap.project.timeline.main[1]!.source_in).toBe(30 * f);
    snap = await run("clip.delete", { clip_id: c2!.id });
    expect(snap.project.timeline.main.map((c) => c.id)).toEqual([c1!.id]);
    expect(mainStarts(snap.project.timeline.main)).toEqual([0]);
  });

  it("ripple trim to the playhead", async () => {
    const { run, a } = await setup();
    let snap = await run("timeline.append", { media_id: a.id });
    const len = clipLength(snap.project.timeline.main[0]!);
    const sec = 30 * frameTicks(snap.project.canvas.frame_rate);
    snap = await run("clip.ripple_trim", { at: sec, edge: "start" });
    expect(clipLength(snap.project.timeline.main[0]!)).toBe(len - sec);
    snap = await run("clip.ripple_trim", { at: sec, edge: "end" });
    expect(clipLength(snap.project.timeline.main[0]!)).toBe(sec);
  });

  it("titles and music: add, style, gain and fades", async () => {
    const { run, song } = await setup();
    let snap = await run("title.add", { start: 0, duration: 3 * S, text: "Hello" });
    const t = snap.project.timeline.titles[0]!;
    snap = await run("title.update", { id: t.id, text: "Hi", style: { size: 96 } });
    expect(snap.project.timeline.titles[0]).toMatchObject({ text: "Hi", style: { size: 96, font: "Space Grotesk" } });
    snap = await run("music.add", { media_id: song.id, start: 0 });
    const m = snap.project.timeline.music[0]!;
    snap = await run("music.update", { id: m.id, gain_db: -6, fade_in: 2 * S, fade_out: 9999 * S });
    expect(snap.project.timeline.music[0]).toMatchObject({ gain_db: -6, fade_in: 2 * S, fade_out: clipLength(m) });
    expect(snap.undo_label).toBe("Music volume and fades");
  });

  it("errors are { code, message } and leave the project unchanged", async () => {
    const { b, run } = await setup();
    await expect(run("clip.split", { at: 0 })).rejects.toMatchObject({ code: "invalid" });
    await expect(run("history.undo", {})).rejects.toMatchObject({ code: "nothing_to_undo" });
    await expect(b.invoke("export_start", { preset: "tiktok", path: "x.mp4" })).rejects.toMatchObject({ code: "empty_timeline" });
  });

  it("export emits progress then done, and can be cancelled", async () => {
    vi.useFakeTimers();
    const { b, run, a } = await setup();
    await run("timeline.append", { media_id: a.id });
    const progress: number[] = [];
    const done = vi.fn();
    await b.listen("export-progress", (p) => progress.push(p.progress));
    await b.listen("export-done", done);
    const job = await b.invoke<string>("export_start", { preset: "youtube-1080", path: "out.mp4" });
    await vi.advanceTimersByTimeAsync(20);
    expect(progress.at(-1)).toBe(100);
    expect(done).toHaveBeenCalledWith({ job, path: "out.mp4" });
    const job2 = await b.invoke<string>("export_start", { preset: "youtube-1080", path: "out2.mp4" });
    await b.invoke("export_cancel", { job: job2 });
    await vi.advanceTimersByTimeAsync(20);
    expect(done).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
  });

  it("save adds to recent projects and reopens", async () => {
    const { b, run, a } = await setup();
    await run("timeline.append", { media_id: a.id });
    await b.invoke("project_save", {});
    const recent = await b.invoke<{ path: string }[]>("recent_projects");
    expect(recent[0]!.path).toBe("Test.editelier");
    await b.invoke("project_new", { name: "Other", aspect: "1:1" });
    const snap = await b.invoke<Snapshot>("project_open", { path: "Test.editelier" });
    expect(snap.project.timeline.main).toHaveLength(1);
  });
});
