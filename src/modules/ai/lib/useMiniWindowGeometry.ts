import { useCallback, useLayoutEffect, useRef } from "react";
import {
  applyDrag,
  applyResize,
  clampGeom,
  defaultGeom,
  snapToSide,
  snapToEdges,
  type Geom,
  type ResizeDir,
  type SnapSide,
  type Viewport,
} from "./miniWindowGeometry";

const STORE_KEY = "terax-ui-mini-window-geom";

const viewport = (): Viewport => ({
  vw: window.innerWidth,
  vh: window.innerHeight,
});

function loadGeom(): Geom | null {
  try {
    const raw = window.localStorage.getItem(STORE_KEY);
    if (!raw) return null;
    const p = JSON.parse(raw) as Partial<Geom>;
    if (
      typeof p.x === "number" &&
      typeof p.y === "number" &&
      typeof p.w === "number" &&
      typeof p.h === "number"
    ) {
      return { x: p.x, y: p.y, w: p.w, h: p.h };
    }
  } catch {
    // corrupt entry — fall back to default placement
  }
  return null;
}

function saveGeom(g: Geom) {
  try {
    window.localStorage.setItem(STORE_KEY, JSON.stringify(g));
  } catch {
    // private mode / quota — geometry just won't persist
  }
}

type Compute = (start: Geom, dx: number, dy: number, vp: Viewport) => Geom;
type Finalize = (current: Geom, vp: Viewport) => Geom;

/** Drives the mini window's position and size entirely through the DOM (no
 * React state), so neither chat streaming nor any other re-render can disturb
 * an in-flight gesture. Writes are batched into a single rAF per frame. */
export function useMiniWindowGeometry() {
  const ref = useRef<HTMLDivElement>(null);
  const geom = useRef<Geom>({ x: 0, y: 0, w: 0, h: 0 });
  const frame = useRef(0);
  const pending = useRef<Geom | null>(null);

  const flush = useCallback(() => {
    frame.current = 0;
    const el = ref.current;
    const g = pending.current;
    if (!el || !g) return;
    el.style.left = `${g.x}px`;
    el.style.top = `${g.y}px`;
    el.style.width = `${g.w}px`;
    el.style.height = `${g.h}px`;
  }, []);

  const write = useCallback(
    (g: Geom) => {
      geom.current = g;
      pending.current = g;
      if (frame.current === 0) frame.current = requestAnimationFrame(flush);
    },
    [flush],
  );

  useLayoutEffect(() => {
    const g = clampGeom(loadGeom() ?? defaultGeom(viewport()), viewport());
    geom.current = g;
    const el = ref.current;
    if (el) {
      el.style.left = `${g.x}px`;
      el.style.top = `${g.y}px`;
      el.style.width = `${g.w}px`;
      el.style.height = `${g.h}px`;
    }
    // Reclamp into the new viewport; persistence is left to the next gesture
    // since loadGeom re-clamps on startup anyway.
    const onResize = () => write(clampGeom(geom.current, viewport()));
    const onSnap = (ev: Event) => {
      const side = (ev as CustomEvent<SnapSide>).detail;
      if (side !== "left" && side !== "right") return;
      const next = snapToSide(geom.current, viewport(), side);
      write(next);
      saveGeom(next);
    };
    window.addEventListener("resize", onResize);
    window.addEventListener("terax:mini-snap", onSnap as EventListener);
    return () => {
      window.removeEventListener("resize", onResize);
      window.removeEventListener("terax:mini-snap", onSnap as EventListener);
      if (frame.current) cancelAnimationFrame(frame.current);
    };
  }, [write]);

  const beginGesture = useCallback(
    (
      e: React.PointerEvent,
      compute: Compute,
      threshold: number,
      finalize?: Finalize,
    ) => {
      const el = e.currentTarget as HTMLElement;
      const pointerId = e.pointerId;
      const startX = e.clientX;
      const startY = e.clientY;
      const start = geom.current;
      // Don't capture the pointer or call preventDefault until the gesture
      // actually moves past the threshold, so a plain click on the header
      // still reaches its buttons, dropdowns and focus handlers.
      let armed = threshold <= 0;
      if (armed) {
        e.preventDefault();
        el.setPointerCapture?.(pointerId);
        document.body.style.userSelect = "none";
      }

      const onMove = (ev: PointerEvent) => {
        const dx = ev.clientX - startX;
        const dy = ev.clientY - startY;
        if (!armed) {
          if (Math.abs(dx) < threshold && Math.abs(dy) < threshold) return;
          armed = true;
          el.setPointerCapture?.(pointerId);
          document.body.style.userSelect = "none";
        }
        write(compute(start, dx, dy, viewport()));
      };
      const onUp = () => {
        el.removeEventListener("pointermove", onMove);
        el.removeEventListener("pointerup", onUp);
        el.removeEventListener("pointercancel", onUp);
        if (!armed) return;
        el.releasePointerCapture?.(pointerId);
        document.body.style.userSelect = "";
        const finalGeom = finalize ? finalize(geom.current, viewport()) : geom.current;
        write(finalGeom);
        saveGeom(finalGeom);
      };
      el.addEventListener("pointermove", onMove);
      el.addEventListener("pointerup", onUp);
      el.addEventListener("pointercancel", onUp);
    },
    [write],
  );

  const onHeaderPointerDown = useCallback(
    (e: React.PointerEvent) => {
      if (e.button !== 0) return;
      const target = e.target as HTMLElement;
      if (
        target.closest(
          "button, input, select, textarea, a, [role], [data-no-drag]",
        )
      )
        return;
      beginGesture(e, applyDrag, 4, snapToEdges);
    },
    [beginGesture],
  );

  const startResize = useCallback(
    (dir: ResizeDir) => (e: React.PointerEvent) => {
      if (e.button !== 0) return;
      e.stopPropagation();
      beginGesture(e, (start, dx, dy, vp) => applyResize(start, dir, dx, dy, vp), 0);
    },
    [beginGesture],
  );

  return { ref, onHeaderPointerDown, startResize };
}
