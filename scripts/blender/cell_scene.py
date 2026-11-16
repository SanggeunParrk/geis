# pyright: reportAttributeAccessIssue=false, reportOptionalMemberAccess=false, reportArgumentType=false
# The Blender stubs type every node, modifier and enum as its base class or its first
# value; a scene built by hand reaches past them on almost every line.
"""The inside of a cell, for the app's opening screens.

Run with Blender, not with Python:

    blender --background --factory-startup --python scripts/blender/cell_scene.py -- \
        <output folder> [preview|final] [far|near|vesicle|all]

Three pictures come out, laid over each other by the window:

- ``far``: the cytoplasm - a membrane wall far off, crowded macromolecules at every
  depth, filaments, and haze lit from behind, as a microscope never sees it and a
  molecular illustrator always draws it.
- ``near``: a few molecules very close, out of focus, on a transparent background - a
  layer of its own, so the window can move it against ``far`` and the space has depth.
- ``vesicle``: a lipid vesicle, transparent around it: what each world lives in. In two
  halves, ``vesicle-back`` and ``vesicle-front``, so a world is drawn between them.

Everything is made here from primitives and noise, seeded, so the same pictures come out
every time and nothing outside this file is needed to make them again.
"""

from __future__ import annotations

import contextlib
import math
import random
import sys
from pathlib import Path
from typing import Any

import bmesh
import bpy
from mathutils import Euler, Vector, noise

SEED = 7

# Muted, as molecular illustration colours are: what the eye reads first is shape and
# light, and saturated colour at every depth would be noise.
PALETTE: list[tuple[tuple[float, float, float], float]] = [
    ((0.20, 0.52, 0.62), 4.0),  # teal: the crowd
    ((0.30, 0.42, 0.70), 2.5),  # blue
    ((0.82, 0.42, 0.36), 1.2),  # coral
    ((0.90, 0.66, 0.32), 0.8),  # amber
    ((0.58, 0.46, 0.72), 1.0),  # violet
    ((0.75, 0.78, 0.80), 0.7),  # pale
]


def arguments() -> tuple[Path, str, str]:
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    out = Path(argv[0]) if argv else Path.cwd()
    quality = argv[1] if len(argv) > 1 else "preview"
    which = argv[2] if len(argv) > 2 else "all"
    return out.resolve(), quality, which


def node(tree: bpy.types.NodeTree, kind: str, **inputs: object) -> bpy.types.Node:
    """Adds a shader node and sets the inputs named, skipping ones this Blender lacks."""
    made = tree.nodes.new(kind)
    for name, value in inputs.items():
        socket = made.inputs.get(name)
        if socket is not None:
            socket.default_value = value
    return made


def protein_material(name: str, colour: tuple[float, float, float]) -> bpy.types.Material:
    """Soft, slightly translucent, with a sheen at grazing angles: a protein's look."""
    material = bpy.data.materials.new(name)
    material.use_nodes = True
    tree: Any = material.node_tree
    tree.nodes.clear()
    out = node(tree, "ShaderNodeOutputMaterial")
    shader = node(
        tree,
        "ShaderNodeBsdfPrincipled",
        **{
            "Base Color": (*colour, 1.0),
            "Roughness": 0.55,
            "Subsurface Weight": 0.25,
            "Subsurface Radius": Vector((0.6, 0.4, 0.3)),
            "Subsurface Scale": 0.15,
            "Sheen Weight": 0.6,
            "Sheen Roughness": 0.4,
            "Sheen Tint": (0.85, 0.95, 1.0, 1.0),
        },
    )
    # Fine lumps: atoms, at the scale a surface shows them.
    bumps = node(tree, "ShaderNodeTexNoise", Scale=9.0, Detail=4.0, Roughness=0.6)
    bump = node(tree, "ShaderNodeBump", Strength=0.35, Distance=0.05)
    tree.links.new(bumps.outputs["Fac"], bump.inputs["Height"])
    tree.links.new(bump.outputs["Normal"], shader.inputs["Normal"])
    tree.links.new(shader.outputs["BSDF"], out.inputs["Surface"])
    return material


def membrane_material() -> bpy.types.Material:
    """A lipid bilayer seen from inside: a lilac sheet stippled with head groups."""
    material = bpy.data.materials.new("membrane")
    material.use_nodes = True
    tree: Any = material.node_tree
    tree.nodes.clear()
    out = node(tree, "ShaderNodeOutputMaterial")
    shader = node(
        tree,
        "ShaderNodeBsdfPrincipled",
        **{
            "Base Color": (0.46, 0.36, 0.58, 1.0),
            "Roughness": 0.45,
            "Subsurface Weight": 0.5,
            "Subsurface Radius": Vector((1.0, 0.5, 0.8)),
            "Subsurface Scale": 0.4,
            "Sheen Weight": 0.8,
            "Sheen Tint": (0.8, 0.9, 1.0, 1.0),
        },
    )
    heads = node(tree, "ShaderNodeTexVoronoi", Scale=38.0)
    bump = node(tree, "ShaderNodeBump", Strength=0.6, Distance=0.03)
    tree.links.new(heads.outputs["Distance"], bump.inputs["Height"])
    tree.links.new(bump.outputs["Normal"], shader.inputs["Normal"])
    tree.links.new(shader.outputs["BSDF"], out.inputs["Surface"])
    return material


def vesicle_material() -> bpy.types.Material:
    """A bilayer seen edge-on at the rim and face-on in the middle: clear in the middle,
    bright where the eye runs along the membrane, as a soap film is."""
    material = bpy.data.materials.new("vesicle")
    material.use_nodes = True
    tree: Any = material.node_tree
    tree.nodes.clear()
    out = node(tree, "ShaderNodeOutputMaterial")
    clear = node(tree, "ShaderNodeBsdfTransparent")
    gloss = node(tree, "ShaderNodeBsdfGlossy", Color=(0.75, 0.92, 1.0, 1.0), Roughness=0.18)
    glow = node(tree, "ShaderNodeEmission", Color=(0.40, 0.82, 1.0, 1.0), Strength=1.2)
    film = node(tree, "ShaderNodeAddShader")
    tree.links.new(gloss.outputs["BSDF"], film.inputs[0])
    tree.links.new(glow.outputs["Emission"], film.inputs[1])
    rim = node(tree, "ShaderNodeLayerWeight", Blend=0.35)
    # Sharpened, so the film thins quickly away from the rim.
    sharpen = node(tree, "ShaderNodeMath")
    sharpen.operation = "POWER"
    sharpen.inputs[1].default_value = 2.2
    tree.links.new(rim.outputs["Facing"], sharpen.inputs[0])
    mix = node(tree, "ShaderNodeMixShader")
    tree.links.new(sharpen.outputs["Value"], mix.inputs["Fac"])
    tree.links.new(clear.outputs["BSDF"], mix.inputs[1])
    tree.links.new(film.outputs["Shader"], mix.inputs[2])
    tree.links.new(mix.outputs["Shader"], out.inputs["Surface"])
    with contextlib.suppress(AttributeError, TypeError):
        material.blend_method = "BLEND"
    return material


def lumpy(
    name: str, radius: float, lumps: float, seed: int, stretch: Vector | None = None
) -> bpy.types.Mesh:
    """A globular protein: a sphere pushed in and out by noise at two scales."""
    mesh = bpy.data.meshes.new(name)
    shape = bmesh.new()
    bmesh.ops.create_icosphere(shape, subdivisions=4, radius=radius)
    offset = Vector((seed * 13.1, seed * 7.7, seed * 3.3))
    for vertex in shape.verts:
        direction = vertex.co.normalized()
        coarse = noise.fractal(direction * 1.6 + offset, 0.8, 2.0, 3)
        fine = noise.noise(direction * 5.0 + offset)
        vertex.co = direction * radius * (1.0 + lumps * coarse + 0.08 * fine)
        if stretch is not None:
            vertex.co = Vector(a * b for a, b in zip(vertex.co, stretch, strict=True))
    shape.to_mesh(mesh)
    shape.free()
    for polygon in mesh.polygons:
        polygon.use_smooth = True
    return mesh


def ribosome(seed: int) -> bpy.types.Mesh:
    """Two subunits, a large and a small, pressed together."""
    large = lumpy("large", 1.0, 0.28, seed)
    small = lumpy("small", 0.72, 0.3, seed + 1, Vector((1.2, 0.8, 0.9)))
    shape = bmesh.new()
    shape.from_mesh(large)
    moved = bmesh.new()
    moved.from_mesh(small)
    for vertex in moved.verts:
        vertex.co += Vector((0.25, 0.0, 0.85))
    joined = bpy.data.meshes.new(f"ribosome{seed}")
    temporary = bpy.data.meshes.new("t")
    moved.to_mesh(temporary)
    shape.from_mesh(temporary)
    shape.to_mesh(joined)
    for mesh in (large, small, temporary):
        bpy.data.meshes.remove(mesh)
    shape.free()
    moved.free()
    for polygon in joined.polygons:
        polygon.use_smooth = True
    return joined


def collection(name: str) -> bpy.types.Collection:
    made = bpy.data.collections.new(name)
    bpy.context.scene.collection.children.link(made)
    return made


def place(
    into: bpy.types.Collection,
    look: tuple[bpy.types.Mesh, bpy.types.Material],
    at: Vector,
    size: float,
    rng: random.Random,
) -> bpy.types.Object:
    """One molecule: a shape and a colour, somewhere, at a size, turned any way."""
    mesh, material = look
    obj = bpy.data.objects.new(mesh.name, mesh)
    obj.location = at
    obj.scale = (size, size, size)
    obj.rotation_euler = Euler(
        (rng.uniform(0, math.tau), rng.uniform(0, math.tau), rng.uniform(0, math.tau))
    )
    # Per object, so one mesh can be every colour.
    obj.material_slots  # noqa: B018 - touching the slots makes them
    if not mesh.materials:
        mesh.materials.append(material)
    obj.data = mesh
    obj.active_material = material
    with contextlib.suppress(AttributeError, TypeError):
        obj.material_slots[0].link = "OBJECT"
        obj.material_slots[0].material = material
    into.objects.link(obj)
    return obj


def filament(
    into: bpy.types.Collection,
    material: bpy.types.Material,
    start: Vector,
    rng: random.Random,
    radius: float,
) -> None:
    """An actin filament or a microtubule: a long, gently wandering tube."""
    curve = bpy.data.curves.new("filament", "CURVE")
    curve.dimensions = "3D"
    curve.bevel_depth = radius
    curve.bevel_resolution = 4
    spline = curve.splines.new("BEZIER")
    points = 9
    spline.bezier_points.add(points - 1)
    heading = Vector(
        (rng.uniform(-1, 1), rng.uniform(-0.4, 0.6), rng.uniform(-0.6, 0.6))
    ).normalized()
    here = start.copy()
    for point in spline.bezier_points:
        point.co = here
        point.handle_left_type = point.handle_right_type = "AUTO"
        heading = (
            heading
            + Vector((rng.uniform(-0.35, 0.35), rng.uniform(-0.2, 0.2), rng.uniform(-0.35, 0.35)))
        ).normalized()
        here = here + heading * rng.uniform(3.5, 5.0)
    obj = bpy.data.objects.new("filament", curve)
    curve.materials.append(material)
    into.objects.link(obj)


def build_cell(rng: random.Random) -> tuple[bpy.types.Collection, bpy.types.Collection]:
    far = collection("far")
    near = collection("near")
    colours = [colour for colour, _ in PALETTE]
    weights = [weight for _, weight in PALETTE]
    materials = [
        protein_material(f"protein{index}", colour) for index, colour in enumerate(colours)
    ]
    templates = [lumpy(f"globule{seed}", 1.0, rng.uniform(0.15, 0.35), seed) for seed in range(6)]
    templates += [
        lumpy(f"rod{seed}", 1.0, 0.2, 20 + seed, Vector((1.8, 0.7, 0.7))) for seed in range(2)
    ]
    templates += [ribosome(40 + seed) for seed in range(2)]

    def pick() -> bpy.types.Material:
        return rng.choices(materials, weights)[0]

    # The crowd: a cell is a third protein by volume, and a picture of one should feel
    # full - but thicker with distance, and with a clearing in the middle near the camera,
    # where the title stands and the eye can see through to the light at the back.
    placed = 0
    while placed < 300:
        depth = 2.0 + 34.0 * math.sqrt(rng.random())
        spread = (depth + 14.0) * 0.36
        x = rng.uniform(-spread, spread) * 1.6
        z = rng.uniform(-spread, spread)
        across = math.hypot(x / (spread * 1.6), z / spread)
        if depth < 22.0 and across < 0.55 - depth / 60.0:
            continue
        size = rng.uniform(0.35, 1.0) * (1.0 + depth / 30.0)
        place(far, (rng.choice(templates), pick()), Vector((x, depth, z)), size, rng)
        placed += 1

    # Close by and out of focus: a layer of its own, for the window to move.
    # Towards the corners, where they frame the view and leave the middle to the title.
    for index in range(8):
        side = -1 if index % 2 else 1
        depth = rng.uniform(-9.5, -6.0)
        reach = (depth + 14.0) * 0.6
        at = Vector(
            (
                side * rng.uniform(0.55, 1.05) * reach * 1.6,
                depth,
                rng.choice([-1, 1]) * rng.uniform(0.3, 1.0) * reach,
            )
        )
        place(near, (rng.choice(templates), pick()), at, rng.uniform(0.9, 1.6), rng)

    # Filaments running through.
    actin = protein_material("actin", (0.88, 0.62, 0.36))
    tubule = protein_material("tubule", (0.36, 0.72, 0.62))
    for index in range(7):
        # From the sides, so they frame the clearing rather than cross it.
        start = Vector(
            (rng.choice([-1, 1]) * rng.uniform(16, 34), rng.uniform(6, 30), rng.uniform(-18, 18))
        )
        filament(far, actin if index % 3 else tubule, start, rng, 0.18 if index % 3 else 0.42)

    # The membrane: a sheet far off, swelling and dipping, studded with its own proteins.
    bpy.ops.mesh.primitive_grid_add(
        x_subdivisions=160,
        y_subdivisions=160,
        size=150,
        location=(0, 44, 0),
        rotation=(math.pi / 2, 0, 0),
    )
    sheet = bpy.context.active_object
    for vertex in sheet.data.vertices:
        world = vertex.co.copy()
        vertex.co.z += 7.0 * noise.fractal(world * 0.025, 0.9, 2.0, 3) + 0.0012 * (world.x**2)
    sheet.data.materials.append(membrane_material())
    for polygon in sheet.data.polygons:
        polygon.use_smooth = True
    bpy.context.scene.collection.objects.unlink(sheet)
    far.objects.link(sheet)
    for _ in range(70):
        at = Vector((rng.uniform(-55, 55), 41.0 + rng.uniform(-3, 1), rng.uniform(-30, 30)))
        place(far, (rng.choice(templates), pick()), at, rng.uniform(0.9, 1.8), rng)
    return far, near


def lights(scene: bpy.types.Scene) -> None:
    # Warm key from above left, cool rim from behind through the haze, faint fill: name,
    # where it stands, what it points at, its colour, its power, its size.
    rig = (
        ("key", (-18, -6, 16), (0, 12, 0), (1.0, 0.80, 0.62), 2600, 10),
        ("rim", (6, 38, 4), (0, 0, 0), (0.40, 0.72, 1.0), 28000, 24),
        ("fill", (16, -10, -6), (0, 10, 0), (0.45, 0.75, 0.9), 500, 12),
    )
    for name, at, aim, colour, power, size in rig:
        light = bpy.data.lights.new(name, "AREA")
        light.color = colour
        light.energy = power
        light.size = size
        obj = bpy.data.objects.new(name, light)
        obj.location = at
        direction = Vector(aim) - Vector(at)
        obj.rotation_euler = direction.to_track_quat("-Z", "Y").to_euler()
        scene.collection.objects.link(obj)


def world(scene: bpy.types.Scene, haze: bool) -> None:
    sky = bpy.data.worlds.new("cytoplasm")
    scene.world = sky
    sky.use_nodes = True
    tree: Any = sky.node_tree
    tree.nodes.clear()
    out = node(tree, "ShaderNodeOutputWorld")
    background = node(tree, "ShaderNodeBackground", Color=(0.006, 0.012, 0.024, 1.0), Strength=1.0)
    tree.links.new(background.outputs["Background"], out.inputs["Surface"])
    if haze:
        volume = node(
            tree,
            "ShaderNodeVolumePrincipled",
            Color=(0.6, 0.8, 1.0, 1.0),
            Density=0.02,
            Anisotropy=0.6,
        )
        tree.links.new(volume.outputs["Volume"], out.inputs["Volume"])


def camera(
    scene: bpy.types.Scene, at: Vector, aim: Vector, optics: tuple[float, float, float]
) -> bpy.types.Object:
    """The camera, and its lens: focal length, focus distance, f-number."""
    lens, focus, stop = optics
    data = bpy.data.cameras.new("camera")
    data.lens = lens
    data.dof.use_dof = True
    data.dof.focus_distance = focus
    data.dof.aperture_fstop = stop
    obj = bpy.data.objects.new("camera", data)
    obj.location = at
    obj.rotation_euler = (aim - at).to_track_quat("-Z", "Y").to_euler()
    scene.collection.objects.link(obj)
    scene.camera = obj
    return obj


def choose_device(scene: bpy.types.Scene) -> None:
    cycles: Any = scene.cycles  # type: ignore[attr-defined]
    addon = bpy.context.preferences.addons.get("cycles")
    preferences: Any = addon.preferences if addon is not None else None
    cycles.device = "CPU"
    if preferences is None:
        return
    with contextlib.suppress(TypeError, ValueError, RuntimeError):
        preferences.compute_device_type = "METAL"
        preferences.get_devices()
        found = [entry for entry in preferences.devices if entry.type == "METAL"]
        if found:
            for entry in preferences.devices:
                entry.use = entry.type == "METAL"
            cycles.device = "GPU"


def configure(
    scene: bpy.types.Scene, width: int, height: int, samples: int, transparent: bool
) -> None:
    scene.render.engine = "CYCLES"
    choose_device(scene)
    cycles: Any = scene.cycles  # type: ignore[attr-defined]
    cycles.samples = samples
    cycles.use_denoising = True
    cycles.max_bounces = 6
    cycles.volume_bounces = 0
    cycles.volume_step_rate = 2.0
    scene.render.resolution_x = width
    scene.render.resolution_y = height
    scene.render.resolution_percentage = 100
    scene.render.film_transparent = transparent
    # WebP: a backdrop is seen whole and at once, and a PNG of it was megabytes.
    scene.render.image_settings.file_format = "WEBP"
    scene.render.image_settings.color_mode = "RGBA" if transparent else "RGB"
    scene.render.image_settings.quality = 86
    with contextlib.suppress(TypeError):
        scene.view_settings.view_transform = "AgX"
    with contextlib.suppress(TypeError):
        scene.view_settings.look = "AgX - Medium High Contrast"


def render(scene: bpy.types.Scene, path: Path) -> None:
    scene.render.filepath = str(path)
    bpy.ops.render.render(write_still=True)
    print(f"[cell] wrote {path}", flush=True)


def cell(out: Path, quality: str, which: str) -> None:
    final = quality == "final"
    width, height = (2880, 1800) if final else (960, 600)
    scene = bpy.context.scene
    rng = random.Random(SEED)  # noqa: S311 - seeded scenery, not a secret
    far, near = build_cell(rng)
    lights(scene)
    # Opened far past any real lens: the scene is metres where a cell is nanometres, and
    # the blur that says "small" has to be asked for.
    camera(scene, Vector((0, -14, 1.5)), Vector((0, 20, 0)), (30, 30.0, 0.06))

    if which in ("far", "all"):
        world(scene, haze=True)
        configure(scene, width, height, 256 if final else 32, transparent=False)
        near.hide_render = True
        far.hide_render = False
        render(scene, out / "cell-far.webp")
    if which in ("near", "all"):
        world(scene, haze=False)
        configure(scene, width, height, 128 if final else 24, transparent=True)
        near.hide_render = False
        far.hide_render = True
        render(scene, out / "cell-near.webp")


def hemisphere(name: str, front: bool, material: bpy.types.Material) -> bpy.types.Object:
    """Half the vesicle's membrane: the half nearer the camera, or the half beyond."""
    mesh = bpy.data.meshes.new(name)
    shape = bmesh.new()
    bmesh.ops.create_uvsphere(shape, u_segments=128, v_segments=64, radius=4.0)
    # The camera looks along +y, so the near half is the one with y below zero.
    doomed = [
        vertex for vertex in shape.verts if (vertex.co.y > 1e-4 if front else vertex.co.y < -1e-4)
    ]
    bmesh.ops.delete(shape, geom=doomed, context="VERTS")
    shape.to_mesh(mesh)
    shape.free()
    for polygon in mesh.polygons:
        polygon.use_smooth = True
    mesh.materials.append(material)
    obj = bpy.data.objects.new(name, mesh)
    solid = obj.modifiers.new("bilayer", "SOLIDIFY")
    solid.thickness = 0.06
    return obj


def vesicle(out: Path, quality: str) -> None:
    """The vesicle in two halves, so a world can be drawn between them - inside it."""
    final = quality == "final"
    scene = bpy.context.scene
    rng = random.Random(SEED + 1)  # noqa: S311 - seeded scenery, not a secret
    material = vesicle_material()
    front = collection("vesicle front")
    back = collection("vesicle back")
    front.objects.link(hemisphere("near half", True, material))
    back.objects.link(hemisphere("far half", False, material))
    # Proteins sitting in the membrane, half in and half out, each wholly in one half so
    # none is cut where the halves meet.
    templates = [lumpy(f"embedded{seed}", 1.0, 0.25, 60 + seed) for seed in range(4)]
    materials = [
        protein_material(f"embedded{index}", colour)
        for index, (colour, _) in enumerate(PALETTE[:4])
    ]
    for _ in range(34):
        direction = Vector((rng.gauss(0, 1), rng.gauss(0, 1), rng.gauss(0, 1))).normalized()
        holder = front if direction.y < 0 else back
        look = (rng.choice(templates), rng.choice(materials))
        place(holder, look, direction * 4.0, rng.uniform(0.2, 0.42), rng)
    world(scene, haze=False)
    camera(scene, Vector((0, -15.5, 0)), Vector((0, 0, 0)), (50, 15.5, 8.0))
    size = 1200 if final else 400
    for name, shown, hidden in (("vesicle-back", back, front), ("vesicle-front", front, back)):
        configure(scene, size, size, 256 if final else 32, transparent=True)
        shown.hide_render = False
        hidden.hide_render = True
        render(scene, out / f"{name}.webp")


def main() -> None:
    out, quality, which = arguments()
    out.mkdir(parents=True, exist_ok=True)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    if which in ("far", "near", "all"):
        cell(out, quality, which)
    if which in ("vesicle", "all"):
        bpy.ops.wm.read_factory_settings(use_empty=True)
        lights(bpy.context.scene)
        vesicle(out, quality)


main()
