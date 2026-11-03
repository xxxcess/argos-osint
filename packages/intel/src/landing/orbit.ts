/** Seeded, time-based orbit math; independent of the renderer and frame rate. */
export type Orbiter = {
  id: string;
  age: number;
  radius: number;
  target: number;
  angle: number;
  tilt: number;
  phase: number;
  speed: number;
  x: number;
  y: number;
  z: number;
};
export function spawnOrbiter(seed: number): Orbiter {
  let value = seed >>> 0;
  const random = () => {
    value = (Math.imul(value, 1664525) + 1013904223) >>> 0;
    return value / 4294967296;
  };
  const target = 80 + random() * 80;
  const angle = (Math.floor(random() * 4) * Math.PI) / 2;
  const tilt = random() * Math.PI;
  const phase = random() * Math.PI * 2;
  return position({
    id: String(seed),
    age: 0,
    radius: 600,
    target,
    angle,
    tilt,
    phase,
    speed: 0.05,
    x: 0,
    y: 0,
    z: 0,
  });
}
function position(o: Orbiter): Orbiter {
  const x = Math.cos(o.angle) * o.radius;
  const planar = Math.sin(o.angle) * o.radius;
  const y = planar * Math.cos(o.tilt);
  const z = planar * Math.sin(o.tilt);
  return {
    ...o,
    x: x * Math.cos(o.phase) - y * Math.sin(o.phase),
    y: x * Math.sin(o.phase) + y * Math.cos(o.phase),
    z,
  };
}
const smooth = (t: number) => t * t * (3 - 2 * t);
function angularIntegral(age: number) {
  const t = Math.min(age / 4, 1);
  return 0.05 * age + 1.15 * (4 * (t ** 3 - t ** 4 / 2) + Math.max(age - 4, 0));
}
export function stepOrbiter(o: Orbiter, dt: number): Orbiter {
  const age = o.age + Math.max(0, dt);
  return position({
    ...o,
    age,
    radius: o.target + (600 - o.target) * (1 - smooth(Math.min(age / 4, 1))),
    angle: o.angle + angularIntegral(age) - angularIntegral(o.age),
    speed: 0.05 + 1.15 * smooth(Math.min(age / 4, 1)),
  });
}
