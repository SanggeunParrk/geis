# pyright: reportAttributeAccessIssue=false, reportOptionalMemberAccess=false, reportArgumentType=false
# The Blender stubs type every node, modifier and enum as its base class or its first
# value; a scene built by hand reaches past them on almost every line.
"""The app icon: a protein ribbon drawn as the letter G.

Run with Blender:

    blender --background --factory-startup --python scripts/blender/icon.py -- <output.png>

A ribbon because that is how a protein is drawn - the representation Irving Geis's
paintings made familiar, and the one the app draws by default - and a G for the name.
The ribbon is rendered alone on transparency with its shadow; `assets/branding/tile.swift`
lays it on the rounded tile, so the tile's colour can change without rendering again.
"""

from __future__ import annotations

import contextlib
import math
import sys
from pathlib import Path
from typing import Any

import bpy
from mathutils import Vector

# Cobalt, as the icon was before the rename.
RIBBON = (0.06, 0.16, 0.85)


def output() -> Path:
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    return Path(argv[0] if argv else "icon.png").resolve()


def letter_g() -> list[tuple[float, float, str]]:
    """The G's centre line, in the plane of the icon: from the top right over the top,
    down the left, along the bottom and up the right to the middle, then in along the bar.
    Each point says how its handles are made: round along the bowl, sharp at the corner."""
    points: list[tuple[float, float, str]] = []
    start, end = math.radians(42), math.radians(360)
    steps = 12
    for step in range(steps + 1):
        angle = start + (end - start) * step / steps
        points.append((math.cos(angle), math.sin(angle), "AUTO"))
    # The corner where the bowl turns into the bar, then the bar.
    points[-1] = (1.0, 0.0, "VECTOR")
    points.append((0.32, 0.0, "VECTOR"))
    return points


def ribbon(points: list[tuple[float, float, str]]) -> bpy.types.Object:
    """A flat band along the line, standing in depth, with rounded edges."""
    curve = bpy.data.curves.new("g", "CURVE")
    curve.dimensions = "3D"
    # Carried along the line rather than measured against "up": measured, it flips where
    # the line runs straight up, which is a seam down the left of the G.
    curve.twist_mode = "MINIMUM"
    curve.extrude = 0.40
    curve.bevel_depth = 0.06
    curve.bevel_resolution = 6
    curve.resolution_u = 24
    spline = curve.splines.new("BEZIER")
    spline.bezier_points.add(len(points) - 1)
    for point, (x, z, handles) in zip(spline.bezier_points, points, strict=True):
        point.co = Vector((x, 0.0, z))
        point.handle_left_type = point.handle_right_type = handles
        # The band stands in depth, so it is seen as a band and not as a line.
        point.tilt = math.radians(90)
    obj = bpy.data.objects.new("g", curve)
    bpy.context.scene.collection.objects.link(obj)
    material = bpy.data.materials.new("cobalt")
    material.use_nodes = True
    shader: Any = material.node_tree.nodes.get("Principled BSDF")
    for name, value in {
        "Base Color": (*RIBBON, 1.0),
        "Roughness": 0.38,
        "Coat Weight": 0.2,
        "Coat Roughness": 0.15,
        "Sheen Weight": 0.0,
        "Subsurface Weight": 0.08,
    }.items():
        socket = shader.inputs.get(name)
        if socket is not None:
            socket.default_value = value
    curve.materials.append(material)
    return obj


def stage(scene: bpy.types.Scene) -> None:
    # A floor that only catches the ribbon's shadow, so the shadow falls on the tile.
    bpy.ops.mesh.primitive_plane_add(size=30, location=(0, 2.4, 0), rotation=(math.pi / 2, 0, 0))
    floor = bpy.context.active_object
    floor.is_shadow_catcher = True
    for name, kind, at, power, size in (
        ("key", "AREA", (-3.0, -5.0, 4.0), 450.0, 9.0),
        ("fill", "AREA", (4.0, -4.0, -1.0), 120.0, 5.0),
    ):
        light = bpy.data.lights.new(name, kind)
        light.energy = power
        light.size = size
        obj = bpy.data.objects.new(name, light)
        obj.location = at
        obj.rotation_euler = (Vector((0, 0, 0)) - Vector(at)).to_track_quat("-Z", "Y").to_euler()
        scene.collection.objects.link(obj)
    sky = bpy.data.worlds.new("sky")
    scene.world = sky
    sky.use_nodes = True
    background: Any = sky.node_tree.nodes.get("Background")
    background.inputs["Color"].default_value = (1.0, 1.0, 1.0, 1.0)
    background.inputs["Strength"].default_value = 0.2

    data = bpy.data.cameras.new("camera")
    data.lens = 85
    camera = bpy.data.objects.new("camera", data)
    camera.location = (0.0, -8.6, 2.2)
    camera.rotation_euler = (Vector((0.0, 0.0, -0.02)) - camera.location).to_track_quat("-Z", "Y").to_euler()
    scene.collection.objects.link(camera)
    scene.camera = camera


def render(scene: bpy.types.Scene, path: Path) -> None:
    scene.render.engine = "CYCLES"
    cycles: Any = scene.cycles
    cycles.samples = 256
    cycles.use_denoising = True
    with contextlib.suppress(TypeError, ValueError, RuntimeError, AttributeError):
        preferences: Any = bpy.context.preferences.addons["cycles"].preferences
        preferences.compute_device_type = "METAL"
        preferences.get_devices()
        for device in preferences.devices:
            device.use = device.type == "METAL"
        cycles.device = "GPU"
    scene.render.resolution_x = scene.render.resolution_y = 1024
    scene.render.film_transparent = True
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    with contextlib.suppress(TypeError):
        # Standard, not AgX: an icon's cobalt is to stay cobalt, not drift to pastel.
        scene.view_settings.view_transform = "Standard"
    scene.render.filepath = str(path)
    bpy.ops.render.render(write_still=True)
    print(f"[icon] wrote {path}", flush=True)


def main() -> None:
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    ribbon(letter_g())
    stage(scene)
    render(scene, output())


main()
