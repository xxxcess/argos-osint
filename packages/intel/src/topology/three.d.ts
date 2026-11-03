declare module "three" {
  export class Object3D {
    name: string;
    visible: boolean;
    scale: { setScalar: (value: number) => void };
    traverse(callback: (object: Object3D) => void): void;
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
    dispose(): void;
  }
  export class Line extends Object3D {
    material: object;
    geometry: BufferGeometry;
    constructor(geometry?: BufferGeometry, material?: object);
    computeLineDistances(): this;
  }
  export class Group extends Object3D {}
  export class Mesh extends Object3D {
    geometry: BufferGeometry;
    material: MeshBasicMaterial | MeshBasicMaterial[];
    scale: { setScalar: (value: number) => void };
    constructor(geometry?: object, material?: object);
  }
  export class MeshBasicMaterial {
    color: { set(value: string): void };
    dispose(): void;
    constructor(parameters?: object);
  }
  export class SphereGeometry {
    constructor(
      radius?: number,
      widthSegments?: number,
      heightSegments?: number,
    );
  }
  export class LineBasicMaterial {
    opacity: number;
    constructor(parameters?: object);
    dispose(): void;
  }
  export class LineDashedMaterial {
    opacity: number;
    constructor(parameters?: object);
  }
  export const AdditiveBlending: number;
  export class CanvasTexture {
    constructor(canvas: HTMLCanvasElement);
    dispose(): void;
  }
  export class SpriteMaterial {
    constructor(parameters?: object);
    color: { set(value: string): void };
    map: CanvasTexture | null;
    opacity: number;
    dispose(): void;
  }
  export class Sprite extends Object3D {
    constructor(material: SpriteMaterial);
    material: SpriteMaterial;
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
