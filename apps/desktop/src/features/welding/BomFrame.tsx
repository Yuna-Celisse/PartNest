import { type RefObject } from "react";

/**
 * The EasyEDA interactive BOM is a self-contained viewer that boots WebGL and
 * keeps its own state in storage, so it needs same-origin access inside the
 * frame. The document stays untrusted: it can only read the BOM cache scope,
 * and every selection is re-checked host-side against the session token.
 */
export function BomFrame({ src, frameRef, onLoad }: { src: string; frameRef: RefObject<HTMLIFrameElement>; onLoad?: () => void }) {
  return <iframe ref={frameRef} src={src} onLoad={onLoad} sandbox="allow-scripts allow-same-origin" title="交互式 BOM" />;
}
