# Independent Quiet Orbit clearance verifier

This verifier reads one rendered 800×450 PNG and reproduces the approved fixed geometry. It does not render frames or edit image assets. It requires Python 3, NumPy, SciPy, and Pillow; see `requirements.txt`.

From the repository root:

```bash
python3 assets/brand/reference/quiet-orbit/source/clearance/verify_clearance.py \
  --reference-frame assets/brand/reference/fleetworks-quiet-orbit.png \
  --output .agent-scratch/quiet-orbit/clearance-reproduced.json
```

The default sweep covers 1,440 phases, spaced 0.25° apart, and all five complete triangle silhouettes. It checks filled polygons against the approximate crescent path, then checks every covered output-pixel center in the expanded satellite alpha footprint against the actual painted crescent in the rendered PNG. The latter check uses connected painted-pixel thresholds of 8/255 and 4/255; the isolated background stars and core are excluded. No clearance credit is taken for hiding rear satellites behind the core or crescent.

The source alpha fringe reaches 1.1 pixels beyond a triangle. The verifier includes a conservative circular dilation that contains that fringe after each sprite's anisotropic transform. Pixel-square edge clearance subtracts both pixels' half-diagonals. A further 0.6-pixel engineering reserve describes finite phase spacing; it is explicitly not a formal continuous-time proof.

For core-volume clearance, the complete transformed triangular face is unprojected with the renderer's perspective factor, including its post-projection lift. The report computes the closest point of that face, conservatively extruded by several assumed depth half-thicknesses, to the spherical core. The depth values are stated bounding assumptions for this 2.5D preview, not measured 3D mesh thicknesses.

Geometry constants are specific to the selected baseline and Quiet Orbit. If the renderer's polygons, orbit, scale, projection, or alpha support change, update this independent replica before relying on the report.
