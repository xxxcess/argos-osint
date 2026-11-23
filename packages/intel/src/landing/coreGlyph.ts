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

function crescentSprite() {
  const size = 224;
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = size;
  const context = canvas.getContext("2d");
  if (!context) return null;

  // The 144×176 mark is offset so its core center sits at the texture center.
  context.translate(40, 51);
  const rock = context.createLinearGradient(12, 43, 117, 167);
  rock.addColorStop(0, "#66717F");
  rock.addColorStop(0.47, "#252D38");
  rock.addColorStop(1, "#101319");
  context.beginPath();
  context.arc(72, 105, 62, 0, Math.PI * 2);
  context.fillStyle = rock;
  context.fill();
  context.strokeStyle = "#566170";
  context.lineWidth = 2;
  context.stroke();
  context.globalCompositeOperation = "destination-out";
  context.beginPath();
  context.arc(72, 69, 50, 0, Math.PI * 2);
  context.fillStyle = "#000";
  context.fill();

  const sprite = new Sprite(
    new SpriteMaterial({
      map: new CanvasTexture(canvas),
      transparent: true,
      depthWrite: false,
    }),
  );
  // In the source mark the sphere radius is 40, while this scene uses 14.
  sprite.scale.setScalar((size * 14) / 40);
  return sprite;
}

function haloSprite() {
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = 64;
  const context = canvas.getContext("2d");
  if (!context) return null;
  const gradient = context.createRadialGradient(32, 32, 0, 32, 32, 32);
  gradient.addColorStop(0, intelColors.text);
  gradient.addColorStop(1, "transparent");
  context.fillStyle = gradient;
  context.fillRect(0, 0, 64, 64);
  const halo = new Sprite(
    new SpriteMaterial({
      map: new CanvasTexture(canvas),
      color: landingColors.core,
      blending: AdditiveBlending,
      transparent: true,
      opacity: 0.55,
      depthWrite: false,
    }),
  );
  halo.scale.setScalar(90);
  return halo;
}

export function coreGlyph() {
  const group = new Group();
  const sphereMaterial = new MeshBasicMaterial({ color: landingColors.core });
  const sphere = new Mesh(new SphereGeometry(14, 20, 16), sphereMaterial);
  const halo = haloSprite();
  const crescent = crescentSprite();
  group.add(sphere);
  if (halo) group.add(halo);
  if (crescent) group.add(crescent);

  function setHalo(scale: number, opacity: number) {
    if (!halo) return;
    halo.scale.setScalar(scale);
    halo.material.opacity = opacity;
  }

  return {
    group,
    setHaloColor(color: string) {
      halo?.material.color.set(color);
    },
    settleHalo() {
      setHalo(90, 0.55);
    },
    pulseHalo(elapsed: number) {
      const pulse = Math.sin((elapsed * Math.PI * 2) / 2.4);
      setHalo(90 + pulse * 12, 0.5 + pulse * 0.12);
    },
    dispose() {
      sphere.geometry.dispose();
      sphereMaterial.dispose();
      for (const sprite of [halo, crescent]) {
        sprite?.material.map?.dispose();
        sprite?.material.dispose();
      }
    },
  };
}
