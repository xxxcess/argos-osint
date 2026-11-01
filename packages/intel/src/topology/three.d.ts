declare module "three" {
  export class Object3D {
    name: string;
    children: Object3D[];
    position: { set: (x: number, y: number, z: number) => void };
    add(object: Object3D): void;
    addEventListener(type: string, listener: () => void): void;
  }
  export class Vector3 {
    constructor(x?: number, y?: number, z?: number);
  }
  export class BufferGeometry {
    setFromPoints(points: Vector3[]): this;
  }
  export class Line extends Object3D {
    geometry: BufferGeometry;
    constructor(geometry?: BufferGeometry, material?: object);
    computeLineDistances(): this;
  }
  export class Group extends Object3D {}
  export class Mesh extends Object3D {
    scale: { setScalar: (value: number) => void };
    constructor(geometry?: object, material?: object);
  }
  export class MeshBasicMaterial {
    constructor(parameters?: object);
  }
  export class SphereGeometry {
    constructor(radius?: number);
  }
  export class LineDashedMaterial {
    constructor(parameters?: object);
  }
  export class AxesHelper extends Object3D {
    constructor(size?: number);
  }
}

declare module "three/examples/jsm/renderers/CSS2DRenderer.js" {
  import { Object3D } from "three";
  export class CSS2DObject extends Object3D {
    constructor(element?: HTMLElement);
  }
  export class CSS2DRenderer {
    domElement: HTMLDivElement;
    render(scene: object, camera: object): void;
    constructor();
  }
}
