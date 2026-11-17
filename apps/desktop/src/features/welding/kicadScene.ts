import * as THREE from "three";

export function pickedReference(object: THREE.Object3D, references: ReadonlySet<string>): string | null {
  for (let node: THREE.Object3D | null = object; node; node = node.parent) {
    if (references.has(node.name)) return node.name;
  }
  return null;
}

/** Board layers stay legible regardless of viewing direction or scene lights. */
export function prepareBoardMaterials(model: THREE.Object3D, meshName: (object: THREE.Object3D) => string) {
  const oldMaterials = new Set<THREE.Material>();
  const retained = new Set<THREE.Material>();
  model.traverse((object) => {
    if (!(object instanceof THREE.Mesh)) return;
    const name = meshName(object);
    const materials = Array.isArray(object.material) ? object.material : [object.material];
    if (!/_(?:pad|silkscreen|soldermask|PCB)$/i.test(name)) {
      materials.forEach((material) => retained.add(material));
      return;
    }
    const silk = /_silkscreen$/i.test(name);
    const convert = (material: THREE.Material) => {
      oldMaterials.add(material);
      const source = material as THREE.MeshStandardMaterial;
      const flat = new THREE.MeshBasicMaterial({
        color: source.color?.clone() ?? new THREE.Color(0xffffff), map: source.map ?? null,
        opacity: silk ? 1 : source.opacity, transparent: silk || source.transparent,
        side: THREE.DoubleSide, depthWrite: silk ? false : source.depthWrite,
        polygonOffset: silk, polygonOffsetFactor: silk ? -1 : 0, polygonOffsetUnits: silk ? -1 : 0,
        toneMapped: false,
      });
      return flat;
    };
    object.material = Array.isArray(object.material) ? materials.map(convert) : convert(materials[0]);
    if (silk) object.renderOrder = 3;
  });
  for (const material of oldMaterials) if (!retained.has(material)) material.dispose();
}

export function topViewDistance(bounds: THREE.Box3, aspect: number, fovDegrees: number) {
  const extent = bounds.getSize(new THREE.Vector3());
  const halfFov = THREE.MathUtils.degToRad(fovDegrees) / 2;
  return Math.max(extent.z / 2, extent.x / (2 * Math.max(aspect, 0.01))) / Math.tan(halfFov) * 1.12 + extent.y;
}

export function orientBoardCamera(camera: THREE.Camera, target: THREE.Vector3, distance: number, side: "top" | "bottom") {
  camera.up.set(0, 0, -1);
  camera.position.copy(target).add(new THREE.Vector3(0, side === "top" ? distance : -distance, 0));
  camera.lookAt(target);
}
