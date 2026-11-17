import { expect, it } from "vitest";
import * as THREE from "three";
import { orientBoardCamera, pickedReference, prepareBoardMaterials, topViewDistance } from "./kicadScene";

it("flips between top and bottom without changing zoom or the focal point", () => {
  const camera = new THREE.PerspectiveCamera();
  const target = new THREE.Vector3(0.1, 0.002, 0.05);
  orientBoardCamera(camera, target, 0.3, "top");
  expect(camera.position.y).toBeCloseTo(0.302);
  expect(camera.getWorldDirection(new THREE.Vector3()).y).toBeCloseTo(-1);
  orientBoardCamera(camera, target, 0.3, "bottom");
  expect(camera.position.y).toBeCloseTo(-0.298);
  expect(camera.position.distanceTo(target)).toBeCloseTo(0.3);
  expect(camera.getWorldDirection(new THREE.Vector3()).y).toBeCloseTo(1);
  expect(target.toArray()).toEqual([0.1, 0.002, 0.05]);
});

it("picks the reference on the component parent, excluding board geometry and unknown nodes", () => {
  const component = new THREE.Group(); component.name = "R1";
  const shape = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshStandardMaterial());
  shape.name = "R_0402_1005Metric"; component.add(shape);
  expect(pickedReference(shape, new Set(["R1"]))).toBe("R1");
  expect(pickedReference(shape, new Set(["R10"]))).toBeNull();
  expect(pickedReference(new THREE.Mesh(), new Set(["R1"]))).toBeNull();
});

it("renders PCB layers without lighting and raises legible silk while retaining component materials", () => {
  const model = new THREE.Group();
  const material = new THREE.MeshStandardMaterial({ color: 0xffffff, opacity: 0.9, transparent: true });
  const silk = new THREE.Mesh(new THREE.BoxGeometry(), material); silk.name = "silk";
  const component = new THREE.Mesh(new THREE.BoxGeometry(), material); component.name = "component";
  model.add(silk, component);
  prepareBoardMaterials(model, (object) => object === silk ? "board_silkscreen" : "R_0402_1005Metric");
  expect(silk.material).toBeInstanceOf(THREE.MeshBasicMaterial);
  expect(silk.material.opacity).toBe(1);
  expect(silk.material.depthWrite).toBe(false);
  expect(silk.material.polygonOffset).toBe(true);
  expect(silk.renderOrder).toBe(3);
  expect(component.material).toBe(material);
});

it("fits a front top view by board dimensions and the viewport aspect ratio", () => {
  const box = new THREE.Box3(new THREE.Vector3(0, 0, 0), new THREE.Vector3(0.1, 0.01, 0.06));
  const distance = topViewDistance(box, 2, 40);
  const visibleHeight = (distance - 0.01) * Math.tan(THREE.MathUtils.degToRad(20)) * 2;
  expect(visibleHeight).toBeGreaterThan(0.06);
  expect(visibleHeight * 2).toBeGreaterThan(0.1);
  expect(topViewDistance(box, 0.5, 40)).toBeGreaterThan(distance);
});
