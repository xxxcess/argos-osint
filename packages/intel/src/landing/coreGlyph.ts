import {
  AdditiveBlending,
  CanvasTexture,
  Group,
  Mesh,
  MeshBasicMaterial,
  SphereGeometry,
  Sprite,
  SpriteMaterial,
} from "three";
import { intelColors, landingColors } from "../shell/intel-theme";
export function coreGlyph(
  core: boolean,
  alternate = false,
  coreColor = landingColors.core,
) {
  const color = core
    ? coreColor
    : alternate
      ? landingColors.orbiterAlt
      : landingColors.orbiter;
  const group = new Group();
  group.add(
    new Mesh(
      new SphereGeometry(core ? 14 : 4, 20, 16),
      new MeshBasicMaterial({ color }),
    ),
  );
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = 64;
  const context = canvas.getContext("2d");
  if (context) {
    const gradient = context.createRadialGradient(32, 32, 0, 32, 32, 32);
    gradient.addColorStop(0, intelColors.text);
    gradient.addColorStop(1, "transparent");
    context.fillStyle = gradient;
    context.fillRect(0, 0, 64, 64);
    const halo = new Sprite(
      new SpriteMaterial({
        map: new CanvasTexture(canvas),
        color,
        blending: AdditiveBlending,
        transparent: true,
        opacity: 0.55,
        depthWrite: false,
      }),
    );
    halo.scale.setScalar(core ? 90 : 24);
    group.add(halo);
  }
  return group;
}
