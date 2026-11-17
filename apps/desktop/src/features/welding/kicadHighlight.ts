import * as THREE from "three";
import type { ProjectPackage } from "../../app/contracts";

export function createKicadHighlighter(model: THREE.Object3D, scene: THREE.Scene) {
  const originals = new Map<THREE.Mesh, THREE.Material | THREE.Material[]>();
  const overlays = new THREE.Group();
  scene.add(overlays);
  const scale = Math.max(new THREE.Box3().setFromObject(model).getSize(new THREE.Vector3()).length(), 0.001);
  const clear = () => {
    for (const [mesh, original] of originals) {
      for (const material of Array.isArray(mesh.material) ? mesh.material : [mesh.material]) material.dispose();
      mesh.material = original;
    }
    originals.clear();
    overlays.traverse((object) => {
      if (object instanceof THREE.Mesh || object instanceof THREE.LineSegments) {
        object.geometry.dispose();
        for (const material of Array.isArray(object.material) ? object.material : [object.material]) material.dispose();
      }
    });
    overlays.clear();
  };
  return {
    select(references: string[], positions: ProjectPackage["positions"] = []) {
      clear();
      const unmodeled: string[] = [];
      for (const reference of new Set(references)) {
        const component = model.getObjectByName(reference);
        let meshes = 0;
        component?.traverse((object) => {
          if (!(object instanceof THREE.Mesh)) return;
          meshes++;
          if (originals.has(object)) return;
          originals.set(object, object.material);
          const highlight = (material: THREE.Material) => {
            const clone = material.clone();
            if ("color" in clone && clone.color instanceof THREE.Color) clone.color.set(0xffc400);
            if ("emissive" in clone && clone.emissive instanceof THREE.Color) {
              clone.emissive.set(0xffa000);
              if ("emissiveIntensity" in clone) clone.emissiveIntensity = 1.5;
            }
            return clone;
          };
          object.material = Array.isArray(object.material) ? object.material.map(highlight) : highlight(object.material);
        });
        if (meshes && component) {
          const box = new THREE.Box3().setFromObject(component).expandByScalar(scale * 0.002);
          const outline = new THREE.Box3Helper(box, 0xffc400);
          for (const material of Array.isArray(outline.material) ? outline.material : [outline.material]) {
            material.depthTest = false;
            material.transparent = true;
          }
          outline.renderOrder = 10;
          overlays.add(outline);
        } else {
          unmodeled.push(reference);
          const position = positions.find((p) => p.reference === reference);
          if (position) {
            // KiCad GLB: metres, Y up; placement CSV: mm, negative Y.
            const marker = new THREE.Mesh(new THREE.SphereGeometry(scale * 0.006, 12, 8), new THREE.MeshBasicMaterial({ color: 0xffc400, depthTest: false, transparent: true, opacity: 0.85 }));
            marker.position.set(position.x_mm / 1000, ["bottom", "back"].includes(position.side) ? -0.001 : 0.001, -position.y_mm / 1000);
            marker.renderOrder = 10;
            overlays.add(marker);
          }
        }
      }
      return unmodeled;
    },
    dispose() { clear(); scene.remove(overlays); },
  };
}
