import { useEffect, useRef, useState } from "react";
import * as THREE from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { TrackballControls } from "three/addons/controls/TrackballControls.js";
import { createKicadHighlighter } from "./kicadHighlight";
import type { ProjectPackage } from "../../app/contracts";
import { orientBoardCamera, pickedReference, prepareBoardMaterials, topViewDistance } from "./kicadScene";

export default function Kicad3DViewer({ projectId, load, designators = [], positions, references = [], onSelectDesignators, side = "top", viewRevision = 0, onSideChange }: { projectId: string; load: (id: string) => Promise<ArrayBuffer>; designators?: string[]; positions?: ProjectPackage["positions"]; references?: string[]; onSelectDesignators?: (references: string[]) => void; side?: "top" | "bottom" | "all"; viewRevision?: number; onSideChange?: (side: "top" | "bottom") => void }) {
  const host = useRef<HTMLDivElement>(null);
  const reset = useRef<(() => void) | null>(null);
  const showFace = useRef<((side: "top" | "bottom") => void) | null>(null);
  const sideRef = useRef(side);
  sideRef.current = side;
  const sideChangeRef = useRef(onSideChange);
  sideChangeRef.current = onSideChange;
  useEffect(() => { if (side !== "all") showFace.current?.(side); }, [side, viewRevision]);
  const [status, setStatus] = useState("正在通过 KiCad 生成 3D 模型，首次加载可能需要一些时间…");
  const [retry, setRetry] = useState(0);
  const highlighter = useRef<ReturnType<typeof createKicadHighlighter> | null>(null);
  const currentSelection = useRef({ designators, positions });
  currentSelection.current = { designators, positions };
  const interaction = useRef({ references, onSelectDesignators });
  interaction.current = { references, onSelectDesignators };
  const [unmodeled, setUnmodeled] = useState<string[]>([]);
  const selectionKey = designators.join("\0");
  useEffect(() => {
    setUnmodeled(highlighter.current?.select(currentSelection.current.designators, positions) ?? []);
  }, [selectionKey, positions]);
  useEffect(() => {
    const element = host.current;
    if (!element) return;
    let live = true;
    let renderer: THREE.WebGLRenderer | undefined;
    let controls: TrackballControls | undefined;
    let observer: ResizeObserver | undefined;
    const events = new AbortController();
    const scene = new THREE.Scene();
    setStatus("正在通过 KiCad 生成 3D 模型，首次加载可能需要一些时间…");
    const disposeModel = (model: THREE.Object3D) => model.traverse((object) => {
      if (!(object instanceof THREE.Mesh)) return;
      object.geometry.dispose();
      for (const material of Array.isArray(object.material) ? object.material : [object.material]) {
        for (const value of Object.values(material)) if (value instanceof THREE.Texture) value.dispose();
        material.dispose();
      }
    });
    void load(projectId).then((buffer) => new GLTFLoader().parseAsync(buffer, "")).then((gltf) => {
      if (!live) { disposeModel(gltf.scene); return; }
      renderer = new THREE.WebGLRenderer({ antialias: true });
      renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
      renderer.setClearColor(0x1c222c);
      renderer.domElement.setAttribute("aria-label", "KiCad PCB 3D 预览");
      element.appendChild(renderer.domElement);
      prepareBoardMaterials(gltf.scene, (object) => {
        const index = gltf.parser.associations.get(object)?.meshes;
        return index === undefined ? object.name : gltf.parser.json.meshes[index].name ?? object.name;
      });
      scene.add(gltf.scene);
      highlighter.current = createKicadHighlighter(gltf.scene, scene);
      setUnmodeled(highlighter.current.select(currentSelection.current.designators, currentSelection.current.positions));
      const bounds = new THREE.Box3().setFromObject(gltf.scene);
      const center = bounds.getCenter(new THREE.Vector3());
      const size = Math.max(bounds.getSize(new THREE.Vector3()).length(), 0.001);
      const camera = new THREE.PerspectiveCamera(40, 1, size / 1000, size * 100);
      camera.up.set(0, 0, -1);
      controls = new TrackballControls(camera, renderer.domElement);
      controls.staticMoving = true;
      controls.rotateSpeed = 3;
      controls.target.copy(center);
      controls.minDistance = size * 0.1;
      controls.maxDistance = size * 10;
      const fit = () => {
        const distance = topViewDistance(bounds, camera.aspect, camera.fov);
        orientBoardCamera(camera, center, distance, "top");
        controls?.target.copy(center); controls?.update();
      };
      reset.current = () => { fit(); sideChangeRef.current?.("top"); }; fit();
      showFace.current = (face) => {
        const target = controls?.target ?? center;
        orientBoardCamera(camera, target, camera.position.distanceTo(target), face);
        controls?.update();
      };
      scene.add(new THREE.HemisphereLight(0xffffff, 0x687388, 2.5));
      const light = new THREE.DirectionalLight(0xffffff, 3);
      light.position.copy(center).add(new THREE.Vector3(size, size * 2, size));
      scene.add(light);
      const canvas = renderer.domElement;
      const raycaster = new THREE.Raycaster();
      let down: { x: number; y: number; id: number } | null = null;
      canvas.addEventListener("pointerdown", (event) => {
        down = event.button === 0 ? { x: event.clientX, y: event.clientY, id: event.pointerId } : null;
      }, { signal: events.signal });
      canvas.addEventListener("pointercancel", () => { down = null; }, { signal: events.signal });
      canvas.addEventListener("pointerup", (event) => {
        const start = down; down = null;
        if (!start || start.id !== event.pointerId || event.button !== 0 || Math.hypot(event.clientX - start.x, event.clientY - start.y) > 5) return;
        const rect = canvas.getBoundingClientRect();
        camera.updateMatrixWorld(); gltf.scene.updateWorldMatrix(true, true);
        raycaster.setFromCamera(new THREE.Vector2((event.clientX - rect.left) / rect.width * 2 - 1, -(event.clientY - rect.top) / rect.height * 2 + 1), camera);
        const hit = raycaster.intersectObject(gltf.scene, true)[0];
        const reference = hit && pickedReference(hit.object, new Set(interaction.current.references));
        if (reference) interaction.current.onSelectDesignators?.([reference]);
      }, { signal: events.signal });
      let firstResize = true;
      const resize = () => {
        const width = element.clientWidth, height = element.clientHeight;
        renderer?.setSize(width, height, false);
        camera.aspect = width / Math.max(height, 1); camera.updateProjectionMatrix();
        controls?.handleResize();
        if (firstResize) { fit(); if (sideRef.current !== "all") showFace.current?.(sideRef.current); firstResize = false; }
      };
      observer = new ResizeObserver(resize); observer.observe(element); resize();
      renderer.setAnimationLoop(() => { controls?.update(); renderer?.render(scene, camera); });
      setStatus("");
    }).catch((error) => { if (live) setStatus(`3D 预览加载失败：${String(error)}`); });
    return () => {
      live = false; reset.current = null; showFace.current = null; events.abort(); observer?.disconnect(); controls?.dispose();
      highlighter.current?.dispose(); highlighter.current = null;
      renderer?.setAnimationLoop(null); disposeModel(scene); renderer?.dispose(); renderer?.domElement.remove();
    };
  }, [projectId, load, retry]);
  return <div className="kicad-3d">
    <div className="kicad-3d__surface" ref={host} />
    <div className="welding-canvas-controls"><button className="pn-button" onClick={() => reset.current?.()}>复位 3D 视图</button></div>
    {designators.length > 0 && <div className="kicad-3d__selection" role="status">已选：{designators.join(", ")}{unmodeled.length > 0 && <span> · {unmodeled.join(", ")} 无实体模型，按坐标标记位置</span>}</div>}
    {status && <div className="kicad-3d__status" role="status"><p>{status}</p>{status.startsWith("3D 预览加载失败") && <button className="pn-button" onClick={() => setRetry((value) => value + 1)}>重试 3D 预览</button>}</div>}
  </div>;
}
