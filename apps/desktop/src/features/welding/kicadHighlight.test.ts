import { expect, it } from "vitest";
import * as THREE from "three";
import { createKicadHighlighter } from "./kicadHighlight";

it("highlights exact reference nodes without recoloring shared materials and restores previous selections", () => {
  const model = new THREE.Group(), scene = new THREE.Scene();
  scene.add(model);
  const original = new THREE.MeshStandardMaterial({ color: 0x334455 });
  const resistor = (name: string) => {
    const node = new THREE.Group(); node.name = name;
    const mesh = new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), original);
    node.add(mesh); model.add(node); return mesh;
  };
  const r1 = resistor("R1"), r10 = resistor("R10");
  const highlight = createKicadHighlighter(model, scene);
  expect(highlight.select(["R1"])).toEqual([]);
  expect(r1.material).not.toBe(original);
  expect((r1.material as THREE.MeshStandardMaterial).color.getHex()).toBe(0xffc400);
  expect(r10.material).toBe(original);
  expect(original.color.getHex()).toBe(0x334455);
  highlight.select(["R10"]);
  expect(r1.material).toBe(original);
  expect(r10.material).not.toBe(original);
  highlight.select(["R1", "R10"]);
  expect(r1.material).not.toBe(original);
  expect(r10.material).not.toBe(original);
  highlight.select([]);
  expect(r1.material).toBe(original);
  expect(r10.material).toBe(original);
  highlight.dispose();
  expect(scene.children).toEqual([model]);
});

it("marks the placement of a selected component with no exported body", () => {
  const model = new THREE.Group(), scene = new THREE.Scene();
  const highlight = createKicadHighlighter(model, scene);
  const positions = [{ reference: "TP1", value: "", package: "", x_mm: 80, y_mm: -90, rotation: 0, side: "bottom" }];
  expect(highlight.select(["TP1"], positions)).toEqual(["TP1"]);
  const marker = scene.children[0].children[0];
  expect(marker.position.toArray()).toEqual([0.08, -0.001, 0.09]);
  highlight.select([]);
  expect(scene.children[0].children).toHaveLength(0);
  highlight.dispose();
});
