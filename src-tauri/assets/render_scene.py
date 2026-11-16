"""Renders a molecular scene with Blender.

Run as:  blender -b -P render_scene.py -- <mesh file> <settings json> <output png>

This script is deliberately the thin end of the Blender bridge. It builds no molecular
geometry of its own: the mesh it loads was produced by the same Rust code the interactive
viewer draws, which is the only way a rendered figure and the viewport can be guaranteed
to show the same thing. Its job is lighting, camera, and materials.

Blender's Python API is a moving target across versions, so anything version-specific is
probed rather than assumed.
"""

from __future__ import annotations

import contextlib
import json
import math
import struct
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import bpy
import mathutils
import numpy as np

MESH_MAGIC = b"MMSH"
MESH_VERSION = 1

# The settings file is JSON written by `render::RenderSettings`; every key is optional
# here so an older app talking to a newer script (or the reverse) still renders.
Settings = dict[str, Any]
Triple = list[float] | tuple[float, float, float]


@dataclass
class Mesh:
    """The flat buffers of the mesh file, as `foreach_set` wants them."""

    positions: np.ndarray
    normals: np.ndarray
    colors: np.ndarray
    indices: np.ndarray
    vertex_count: int
    triangle_count: int


def srgb_to_linear(color: Triple) -> list[float]:
    """sRGB 0..1 to linear.

    Blender's colour inputs are linear, and the app sends the background as the sRGB value
    the user picked. Feeding sRGB straight in renders a noticeably darker, more saturated
    ground than the one chosen.
    """
    return [
        value / 12.92 if value <= 0.04045 else ((value + 0.055) / 1.055) ** 2.4 for value in color
    ]


def log(message: str) -> None:
    # Blender's stdout is captured by the caller and shown in the app, so this is the
    # progress channel.
    print(f"[render] {message}", flush=True)


def node_tree(owner: bpy.types.Material | bpy.types.World) -> bpy.types.NodeTree:
    """The owner's node tree, emptied.

    `node_tree` is None until nodes are switched on, so this is the one place that both
    switches them on and checks that it took.
    """
    owner.use_nodes = True
    tree = owner.node_tree
    if tree is None:
        raise SystemExit(f"Blender gave {owner.name} no node tree")
    tree.nodes.clear()
    return tree


def add_node(tree: bpy.types.NodeTree, kind: str) -> Any:  # noqa: ANN401
    """A new node, untyped on purpose.

    Sockets and node properties are looked up by name at run time, and the names differ
    between Blender releases; a static type would only describe one of them.
    """
    return tree.nodes.new(kind)


def set_enum(owner: object, name: str, value: str) -> None:
    """Assigns an enum that add-ons extend at run time.

    The engine and colour-management lists are filled in when Blender starts, so the only
    way to learn whether a value exists is to assign it; a missing one raises TypeError.
    """
    setattr(owner, name, value)


def read_mesh(path: str) -> Mesh:
    """Reads the binary mesh written by `geometry::tessellate::encode_mesh_file`."""
    blob = Path(path).read_bytes()
    if len(blob) < 16:
        raise SystemExit(f"{path} is too short to be a mesh file")

    magic, version, vertex_count, triangle_count = struct.unpack_from("<4sIII", blob, 0)
    if magic != MESH_MAGIC:
        raise SystemExit(f"{path} is not a mesh file (magic {magic!r})")
    if version != MESH_VERSION:
        raise SystemExit(f"{path} is mesh version {version}, this script reads {MESH_VERSION}")

    offset = 16
    floats = vertex_count * 3

    def take(count: int, dtype: type) -> np.ndarray:
        nonlocal offset
        if offset + count * np.dtype(dtype).itemsize > len(blob):
            raise SystemExit(f"{path} is truncated: it ends before its own counts say it should")
        array = np.frombuffer(blob, dtype=dtype, count=count, offset=offset)
        offset += count * np.dtype(dtype).itemsize
        return array

    positions = take(floats, np.float32)
    normals = take(floats, np.float32)
    colors = take(floats, np.float32)
    indices = take(triangle_count * 3, np.uint32)
    log(f"{vertex_count} vertices, {triangle_count} triangles")
    return Mesh(positions, normals, colors, indices, vertex_count, triangle_count)


def build_object(
    scene: bpy.types.Scene, data: Mesh, material: bpy.types.Material
) -> bpy.types.Object:
    """Builds the mesh with the bulk API.

    `from_pydata` is the documented way and is unusable here: it walks Python lists, and a
    400,000-triangle cartoon takes minutes. `foreach_set` writes flat buffers straight into
    Blender's arrays.
    """
    vertex_count, triangle_count = data.vertex_count, data.triangle_count
    mesh = bpy.data.meshes.new("structure")
    mesh.vertices.add(vertex_count)
    mesh.vertices.foreach_set("co", data.positions)

    loop_total = triangle_count * 3
    mesh.loops.add(loop_total)
    mesh.loops.foreach_set("vertex_index", data.indices)

    mesh.polygons.add(triangle_count)
    mesh.polygons.foreach_set("loop_start", np.arange(0, loop_total, 3, dtype=np.int32))
    mesh.polygons.foreach_set("loop_total", np.full(triangle_count, 3, dtype=np.int32))

    # Faces have to be smooth *before* the custom normals go on: Blender only uses custom
    # split normals across edges it considers smooth, and a flat-shaded face takes its
    # normal from the face - which is what makes a tessellated cylinder read as a stack of
    # flat slabs. Blender 5.2 already defaults `use_smooth` to True, but this area of the
    # API changed twice across 4.x, so it is stated rather than assumed.
    mesh.polygons.foreach_set("use_smooth", np.ones(triangle_count, dtype=bool))
    mesh.update()
    # Custom normals are what keep a sphere looking round at 8 rings rather than faceted.
    # Typed as nested sequences, but it takes the array as it is, which is what keeps a
    # large mesh fast.
    normals: Any = data.normals.reshape(-1, 3)
    mesh.normals_split_custom_set_from_vertices(normals)

    # Vertex colours carry the whole scene's colouring, so one material covers a cartoon,
    # its ligands, and their bonds.
    attribute = mesh.color_attributes.new(name="Col", type="FLOAT_COLOR", domain="POINT")
    with_alpha = np.ones((vertex_count, 4), dtype=np.float32)
    with_alpha[:, :3] = data.colors.reshape(-1, 3)
    if not isinstance(attribute, bpy.types.FloatColorAttribute):
        raise SystemExit("Blender did not create a float colour attribute")
    attribute.data.foreach_set("color", with_alpha.reshape(-1))

    mesh.materials.append(material)
    obj = bpy.data.objects.new("structure", mesh)
    scene.collection.objects.link(obj)
    return obj


def make_material(settings: Settings) -> bpy.types.Material:
    material = bpy.data.materials.new("structure")
    tree = node_tree(material)

    output = add_node(tree, "ShaderNodeOutputMaterial")
    principled = add_node(tree, "ShaderNodeBsdfPrincipled")
    # ShaderNodeAttribute rather than ShaderNodeVertexColor: the latter's socket names have
    # changed between releases, this one has not.
    attribute = add_node(tree, "ShaderNodeAttribute")
    attribute.attribute_name = "Col"

    principled.inputs["Roughness"].default_value = float(settings.get("roughness", 0.28))
    # Written by name and skipped when absent: Principled's socket set has changed several
    # times across releases, and a KeyError here would fail the whole render over a
    # cosmetic parameter.
    optional = (
        ("Metallic", float(settings.get("metallic", 0.0))),
        ("IOR", 1.45),
        ("Specular IOR Level", float(settings.get("specular", 0.5))),
    )
    for socket, value in optional:
        if socket in principled.inputs:
            principled.inputs[socket].default_value = value

    tree.links.new(attribute.outputs["Color"], principled.inputs["Base Color"])
    tree.links.new(principled.outputs["BSDF"], output.inputs["Surface"])
    return material


def look_at(eye_at: Triple, target_at: Triple, up_along: Triple) -> mathutils.Matrix:
    """Camera matrix from a look-at triple.

    Blender cameras look down their local -Z with +Y up, which is why this builds the basis
    by hand rather than using a track-to constraint: the viewer already has an orientation,
    and the render must match it exactly rather than approximately.
    """
    eye = np.array(eye_at, dtype=np.float64)
    target = np.array(target_at, dtype=np.float64)
    up = np.array(up_along, dtype=np.float64)

    forward = eye - target
    norm = np.linalg.norm(forward)
    forward = forward / norm if norm > 1e-9 else np.array([0.0, 0.0, 1.0])
    right = np.cross(up, forward)
    norm = np.linalg.norm(right)
    right = right / norm if norm > 1e-9 else np.array([1.0, 0.0, 0.0])
    true_up = np.cross(forward, right)

    matrix = np.identity(4)
    matrix[:3, 0] = right
    matrix[:3, 1] = true_up
    matrix[:3, 2] = forward
    matrix[:3, 3] = eye
    # A `mathutils.Matrix`, not a list of lists: assigning a nested list to
    # `matrix_world` is silently ignored - no exception, no warning, the object simply
    # stays at the origin. That produced a perfectly successful render of an empty frame,
    # because the structure was behind the camera.
    return mathutils.Matrix([[float(value) for value in row] for row in matrix])


def new_light(name: str, spec: Settings, radius: float) -> bpy.types.Light:
    """One lamp of the kind asked for; anything unrecognised is a sun."""
    kind = spec.get("kind", "sun")
    energy = float(spec.get("energy", 1.0))
    distance = float(spec.get("distance", 4.0)) * radius
    softness = float(spec.get("softness", 8.0))
    if kind == "point":
        point = bpy.data.lights.new(name, type="POINT")
        assert isinstance(point, bpy.types.PointLight)  # noqa: S101
        # Irradiance at the subject is power / (4 pi d^2), so inverting that keeps the
        # same energy number meaning the same brightness whatever the structure's size.
        point.energy = energy * 4.0 * math.pi * max(distance, 1e-3) ** 2
        return point
    if kind == "area":
        area = bpy.data.lights.new(name, type="AREA")
        assert isinstance(area, bpy.types.AreaLight)  # noqa: S101
        area.energy = energy * 4.0 * math.pi * max(distance, 1e-3) ** 2
        area.size = max(radius * softness * 0.1, 1e-3)
        return area
    sun = bpy.data.lights.new(name, type="SUN")
    assert isinstance(sun, bpy.types.SunLight)  # noqa: S101
    sun.energy = energy
    # A little angular size softens contact shadows; a point-sharp sun looks synthetic.
    sun.angle = math.radians(max(softness, 0.1))
    return sun


def add_lights(scene: bpy.types.Scene, settings: Settings, center: Triple, radius: float) -> None:
    """Builds the rig the app defined.

    The lights come from the same settings the viewport is using, so the preview predicts
    the render rather than merely resembling it.

    Suns are the default kind for a decisive reason: a sun's energy is irradiance in W/m^2,
    independent of distance and of scene scale, while a point or area light's falls off as
    1/d^2. With distance-dependent lights the rig that suits a 10 A ligand leaves a 300 A
    capsid black. Point and area lights are still offered - they are what gives falloff and
    genuinely soft shadows - and their power is scaled by the subject's size so that the
    same energy number means the same brightness at any scale.
    """
    directions_used = []
    for index, spec in enumerate(settings.get("lights", [])):
        color = spec.get("color", [1.0, 1.0, 1.0])
        direction = spec.get("direction", [0.0, 0.0, 1.0])
        distance = float(spec.get("distance", 4.0)) * radius
        name = spec.get("name", f"light{index}")

        light = new_light(name, spec, radius)
        light.color = (color[0], color[1], color[2])

        obj = bpy.data.objects.new(name, light)
        offset = [direction[axis] * distance for axis in range(3)]
        position = [center[axis] + offset[axis] for axis in range(3)]
        obj.matrix_world = look_at(position, center, (0.0, 1.0, 0.0))
        # Cycles renders an area light as a visible rectangle and a point light as a
        # visible sphere. A lamp is not part of the subject, so it lights the scene without
        # appearing in it - otherwise switching a light to Area puts a white slab in frame.
        if hasattr(obj, "visible_camera"):
            obj.visible_camera = False
        scene.collection.objects.link(obj)
        directions_used.append(name)

    if not directions_used:
        # A scene with every light switched off would render as a silhouette; the ambient
        # term is the only thing left, and saying so beats producing a mystery.
        log("no lights are enabled - the render will be lit by ambient light alone")
    else:
        log(f"lights: {', '.join(directions_used)}")


def make_world(background: Triple, ambient: float) -> bpy.types.World:
    """A world that lights the scene without dimming the visible background.

    The obvious version - one Background node at the ambient strength - makes the two
    inseparable: turning the ambient down to keep a figure from washing out also turns a
    white background gray, which is what happened. Splitting on Is Camera Ray gives the
    camera the colour that was picked at full strength, and everything else the same colour
    scaled to the ambient level.
    """
    world = bpy.data.worlds.new("world")
    tree = node_tree(world)

    linear = srgb_to_linear(background)
    color = (linear[0], linear[1], linear[2], 1.0)

    visible = add_node(tree, "ShaderNodeBackground")
    visible.inputs["Color"].default_value = color
    visible.inputs["Strength"].default_value = 1.0

    lighting = add_node(tree, "ShaderNodeBackground")
    lighting.inputs["Color"].default_value = color
    lighting.inputs["Strength"].default_value = ambient

    light_path = add_node(tree, "ShaderNodeLightPath")
    mix = add_node(tree, "ShaderNodeMixShader")
    output = add_node(tree, "ShaderNodeOutputWorld")

    # Mix's first shader input is used when the factor is 0, so lighting goes first and the
    # camera ray selects the visible one.
    tree.links.new(light_path.outputs["Is Camera Ray"], mix.inputs["Fac"])
    tree.links.new(lighting.outputs["Background"], mix.inputs[1])
    tree.links.new(visible.outputs["Background"], mix.inputs[2])
    tree.links.new(mix.outputs["Shader"], output.inputs["Surface"])
    return world


def choose_engine(scene: bpy.types.Scene, requested: str) -> str:
    """Sets a render engine that this Blender actually has, and returns it.

    By assignment rather than by introspection: `render.engine` is a *dynamic* enum, since
    add-ons register engines at runtime, and its `enum_items` comes back empty. Reading it
    to decide what is available silently reports nothing is, which is how a render asked
    for in Cycles quietly came out of EEVEE. Assigning an unavailable identifier raises
    TypeError, so trying is both simpler and correct.
    """
    if requested == "eevee":
        candidates = ["BLENDER_EEVEE_NEXT", "BLENDER_EEVEE", "CYCLES"]
    else:
        candidates = ["CYCLES", "BLENDER_EEVEE_NEXT", "BLENDER_EEVEE"]

    for candidate in candidates:
        with contextlib.suppress(TypeError):
            set_enum(scene.render, "engine", candidate)
            return candidate
    # Whatever it was already set to is at least valid.
    return scene.render.engine


def report_brightness(path: str, background: Triple) -> None:
    """Reports whether the image actually has a subject in it.

    Two failures today rendered a technically successful, entirely empty frame: a camera
    that silently refused to move, and a lighting rig too weak to register. Both are
    invisible in an exit code and obvious in one number, so the number is always logged.
    """
    try:
        image = bpy.data.images.load(path)
        # The stubs describe `pixels` as neither sized nor indexable; it is both.
        stored: Any = image.pixels
        flat = np.empty(len(stored), dtype=np.float32)
        stored.foreach_get(flat)
        pixels = flat.reshape(-1, 4)[:, :3]
    except (RuntimeError, ValueError) as error:
        log(f"could not measure the render: {error}")
        return

    # The setting is already the sRGB value the user picked, and the PNG is sRGB, so they
    # compare directly.
    distance = np.abs(pixels - np.array(background)).max(axis=1)
    subject = float((distance > 0.02).mean())
    log(
        f"brightness: mean {pixels.mean():.3f}, max {pixels.max():.3f}, "
        f"{subject * 100:.1f}% of pixels differ from the background"
    )
    if subject < 0.005:
        log("WARNING: the render is essentially empty - check the camera and the lights")


def place_camera(scene: bpy.types.Scene, settings: Settings, center: Triple, radius: float) -> None:
    """Adds the camera where the viewport's was, and refuses to render if it is not there."""
    width = int(settings.get("width", 1600))
    height = int(settings.get("height", 1200))

    # The field of view arrives already worked out - see `RenderSettings::camera_fov_y`,
    # which is where the "never crop what the viewport showed" rule lives and is tested.
    # What matters here is that it is applied to the *vertical* axis: Blender's `angle`
    # goes to the larger render dimension by default, so a 40 degree vertical view became a
    # 40 degree horizontal one at 1600x1200 and cut the top and bottom off every render.
    camera_data = bpy.data.cameras.new("camera")
    camera_data.sensor_fit = "VERTICAL"
    camera_data.lens_unit = "FOV"
    camera_data.angle_y = math.radians(settings.get("camera_fov_y", 40.0))
    camera_data.clip_start = max(radius * 0.001, 0.01)
    camera_data.clip_end = radius * 100.0
    log(f"camera: vertical fov {math.degrees(camera_data.angle_y):.1f} deg at {width}x{height}")

    requested = settings.get("eye", [0.0, 0.0, radius * 3.0])
    camera = bpy.data.objects.new("camera", camera_data)
    camera.matrix_world = look_at(requested, center, settings.get("up", [0.0, 1.0, 0.0]))
    scene.collection.objects.link(camera)
    scene.camera = camera

    # Verify the camera actually moved. Silent no-ops in this API are the reason this check
    # exists, and an empty render is otherwise indistinguishable from a lighting problem.
    at = camera.matrix_world.translation
    placed = (at.x, at.y, at.z)
    if max(abs(placed[axis] - requested[axis]) for axis in range(3)) > 1e-3:
        raise SystemExit(
            f"the camera did not move: asked for {tuple(requested)}, it is at {placed}"
        )

    # The subject has to be in front of the camera. Checking the sign of the depth catches
    # a mirrored or inverted camera basis, which renders an empty frame just as
    # convincingly as a wrong position. A camera looks down its local -Z.
    forward = camera.matrix_world.to_3x3() @ mathutils.Vector((0.0, 0.0, -1.0))
    to_center = mathutils.Vector(center) - at
    if forward.dot(to_center) <= 0.0:
        raise SystemExit("the camera is facing away from the structure")
    log(f"camera at {tuple(round(v, 2) for v in placed)}, looking at {tuple(center)}")


def choose_cycles_device(scene: bpy.types.Scene) -> str:
    """Puts Cycles on a GPU that is really there, else the CPU; returns which, for the log.

    Naming a backend is not the same as having one. A factory reset leaves the device list
    empty until it is asked for, and a backend with no hardware behind it is accepted
    without complaint - Cycles then renders on the CPU while the log says otherwise. So
    each backend is asked what it found, and only one that found a GPU is used.
    """
    cycles: Any = scene.cycles  # type: ignore[attr-defined]  # added by the Cycles add-on
    addon = bpy.context.preferences.addons.get("cycles")
    preferences: Any = addon.preferences if addon is not None else None
    if preferences is None:
        cycles.device = "CPU"
        return "CPU"

    for device in ("METAL", "OPTIX", "CUDA", "HIP", "ONEAPI"):
        try:
            preferences.compute_device_type = device
            preferences.get_devices()
        except (TypeError, ValueError, RuntimeError):
            continue
        found = [entry for entry in preferences.devices if entry.type == device]
        if not found:
            continue
        for entry in preferences.devices:
            entry.use = entry.type == device
        cycles.device = "GPU"
        return f"{device} ({', '.join(entry.name for entry in found)})"

    with contextlib.suppress(TypeError):
        preferences.compute_device_type = "NONE"
    cycles.device = "CPU"
    return "CPU"


def configure_render(scene: bpy.types.Scene, settings: Settings, output_path: str) -> str:
    """Engine, resolution, output and colour management; returns the engine in use."""
    wanted = settings.get("engine", "cycles")
    engine = choose_engine(scene, wanted)
    if engine == "CYCLES" and wanted == "eevee":
        log("EEVEE is not available in this build; using Cycles")
    elif engine != "CYCLES" and wanted == "cycles":
        log(f"Cycles is not available in this build; using {engine}")

    render = scene.render
    render.resolution_x = int(settings.get("width", 1600))
    render.resolution_y = int(settings.get("height", 1200))
    render.resolution_percentage = 100
    render.image_settings.file_format = "PNG"
    render.filepath = output_path
    # Blender otherwise appends or swaps the extension to match the format, and the
    # caller looks for the file at exactly the path it asked for.
    render.use_file_extension = False
    render.film_transparent = bool(settings.get("transparent", False))

    if engine == "CYCLES":
        cycles: Any = scene.cycles  # type: ignore[attr-defined]  # added by the Cycles add-on
        cycles.samples = int(settings.get("samples", 64))
        cycles.use_denoising = True
        # Ambient occlusion in the contact regions is most of what makes a molecular
        # figure readable, and it is the reason to render outside the viewport at all.
        cycles.use_fast_gi = True
        log(f"cycles device: {choose_cycles_device(scene)}")

    # Standard rather than the default filmic-style transform. AgX rolls off highlights
    # and desaturates them, which is right for photography and wrong for a figure whose
    # colours were chosen deliberately: a yellow sheet must come out the yellow that was
    # asked for.
    try:
        set_enum(scene.view_settings, "view_transform", "Standard")
        set_enum(scene.view_settings, "look", "None")
    except TypeError:
        log("this build has no Standard view transform; colours may be tone mapped")
    return engine


_last_progress = ""


def report_progress(stats: str) -> None:
    """Passes Blender's own progress line on, for the app's gauge.

    Blender run in the background prints nothing between starting a render and saving
    it; the status line it keeps for its own window is the only account of how far it
    has got - "Sample 12/64", or which kernels it is still loading. Only a change is
    passed on.
    """
    global _last_progress  # noqa: PLW0603 - Blender calls this with nothing to hold state in
    line = stats.rsplit("|", 1)[-1].strip() if "Sample" not in stats else stats.strip()
    if line != _last_progress:
        _last_progress = line
        print(f"[progress] {line}", flush=True)


def main() -> None:
    if "--" not in sys.argv:
        raise SystemExit("usage: blender -b -P render_scene.py -- <mesh> <settings> <output>")
    args = sys.argv[sys.argv.index("--") + 1 :]
    if len(args) < 3:
        raise SystemExit("expected a mesh file, a settings file, and an output path")
    mesh_path, settings_path = args[0], args[1]
    # Absolute, because Blender resolves a relative output path against its own idea of
    # the current directory and then fails to save.
    output_path = str(Path(args[2]).resolve())

    settings: Settings = json.loads(Path(settings_path).read_text(encoding="utf-8"))

    # Start from nothing: the factory scene's cube, camera, and light would all appear in
    # the render.
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    if scene is None:
        raise SystemExit("Blender started without a scene")

    mesh = read_mesh(mesh_path)
    if mesh.vertex_count == 0:
        raise SystemExit("the scene has no geometry to render")

    obj = build_object(scene, mesh, make_material(settings))
    corners = [tuple(round(value, 2) for value in corner) for corner in obj.bound_box]
    log(f"geometry spans {corners[0]} to {corners[6]}")

    center = settings.get("center", [0.0, 0.0, 0.0])
    radius = max(float(settings.get("radius", 10.0)), 1e-3)
    background = settings.get("background", [1.0, 1.0, 1.0])

    place_camera(scene, settings, center, radius)
    add_lights(scene, settings, center, radius)
    scene.world = make_world(background, float(settings.get("ambient", 0.55)))
    engine = configure_render(scene, settings, output_path)

    log(f"engine {engine}, {scene.render.resolution_x}x{scene.render.resolution_y}")
    # The stubs describe the handler list as taking scene callbacks; this one is handed
    # the status string, which is what Blender passes to `render_stats`.
    handlers: Any = bpy.app.handlers.render_stats
    handlers.append(report_progress)
    bpy.ops.render.render(write_still=True)
    log(f"wrote {output_path}")
    report_brightness(output_path, background)


if __name__ == "__main__":
    main()
