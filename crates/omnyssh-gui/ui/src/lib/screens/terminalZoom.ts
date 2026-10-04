// Ctrl + mouse wheel over a terminal changes the terminals' font size. Listened for on
// the capture phase and swallowed, so the webview never page-zooms and xterm neither
// scrolls nor reports the wheel to a remote program. Deltas are accumulated so a
// trackpad's small steps and a mouse notch both feel right.
const PX_PER_STEP = 50;
const LINE_PX = 16;
const PAGE_PX = 400;

export function attachWheelZoom(el: HTMLElement, onStep: (delta: 1 | -1) => void): () => void {
  let acc = 0;
  function onWheel(e: WheelEvent): void {
    if (!e.ctrlKey) return;
    e.preventDefault();
    e.stopPropagation();
    const scale = e.deltaMode === 1 ? LINE_PX : e.deltaMode === 2 ? PAGE_PX : 1;
    acc += e.deltaY * scale;
    while (Math.abs(acc) >= PX_PER_STEP) {
      const up = acc < 0;
      onStep(up ? 1 : -1);
      acc += up ? PX_PER_STEP : -PX_PER_STEP;
    }
  }
  el.addEventListener('wheel', onWheel, { passive: false, capture: true });
  return () => el.removeEventListener('wheel', onWheel, { capture: true });
}
