#!/usr/bin/env python3
"""The parity suite: every feature, proven against a real daemon.

One check per feature. Each drives the running engine over
its socket and asserts an effect, never a duration and never a log line. It
needs this machine, a display and its grants, which is why it is not in CI
and is not in `make remuxd.check`.

    make remuxd.smoke            every check
    make remuxd.smoke S=screens  one of them
"""

import glob
import json
import os
import statistics
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import base64  # noqa: E402

from drive import Daemon as WireDaemon  # noqa: E402


class Daemon(WireDaemon):
    """Present the layer-only wire to older parity assertions.

    The adapter exists only in this test harness, never in the daemon or a
    client. First assert that removed fields really are absent; then project
    the unique layer into the old single-source assertions. Multiple matches
    intentionally produce no implicit choice.
    """

    def ask(self, command):
        reply = super().ask(command)
        if reply.get("reply") != "status":
            return reply
        for removed in ("screen", "camera", "camera_position", "camera_shape", "flowing", "camera_flowing"):
            assert removed not in reply, f"legacy wire field {removed} came back"

        def unique(kinds):
            matches = [layer for layer in reply["layers"] if layer["source"]["kind"] in kinds]
            return matches[0] if len(matches) == 1 else None

        display = unique(("screen", "window"))
        camera = unique(("camera",))
        scene = reply["scene_flowing"]
        view = dict(reply)
        view["screen"] = display["source"]["name"] if display else None
        view["camera"] = camera["source"]["name"] if camera else None
        view["camera_flowing"] = reply["layer_flowing"].get(camera["id"], {}) if camera else {
            "captured": 0, "frames": 0, "width": 0, "height": 0
        }
        source = reply["layer_flowing"].get(display["id"], {}) if display else {}
        view["flowing"] = {
            **scene,
            "captured": source.get("captured", 0),
            "width": source.get("width", 0),
            "height": source.get("height", 0),
        }
        return view

# The picture's rate, what every camera is held at (the motors hold it).
OUTPUT_FPS = 30

CHECKS = {}
#: Checks that need the machine to themselves. They start a release daemon of
#: their own and measure timing, and a second engine capturing the same display
#: is not a test of either: measured, the pair produced a file at 6 fps with
#: 689 ms stalls in it and every timing assertion blamed the product. These run
#: after the shared daemon has gone.
ALONE = set()
#: Checks that make a sound in the room, or take the operator's headset away
#: for a moment. Asked for by name, never by default.
# The benchmarks (headto 59 s, sync 33 s) and the relay road (needs `make
# relay.up`) run by name only: the default set is 2 min without them and 4
# with.
NOISY = {"loopback", "replug", "headset", "headto", "sync", "publish", "free", "byo", "go"}


def check(name, alone=False):
    def register(fn):
        CHECKS[name] = fn
        if alone:
            ALONE.add(name)
        return fn

    return register


class Failed(Exception):
    pass


def expect(condition, said):
    if not condition:
        raise Failed(said)


# ---- 1. screens carry the monitor's name ----------------------------------


@check("screens")
def screens(daemon):
    """A screen is named after the monitor, and found by id, not by position.

    The capturer calls them "Screen 1" and "Screen 2"; the display list knows
    them by what is written on the front. The two order the same hardware
    differently, so the join is on the display id and never on position.
    """
    found = daemon.ask({"cmd": "devices"})["screens"]
    expect(found, "no screens at all: is the screen recording grant there?")
    dull = [s["name"] for s in found if s["name"].startswith("Screen ")]
    expect(
        not dull,
        f"these kept the capturer's dull label instead of the monitor's name: {dull}",
    )
    for s in found:
        expect(s["id"].isdigit(), f"a screen id must be a display id, got {s['id']!r}")
    return ", ".join(f"{s['name']} (display {s['id']})" for s in found)


# ---- 2. a source, chosen by name ------------------------------------------


@check("sources")
def sources(daemon):
    """A screen by its display id, a window by part of what it says.

    Both answer with the name of what they chose rather than a bare ok: "screen
    3" and "the one called VG2791R" are different amounts of confidence, and a
    person about to go live wants the second. A miss changes nothing, because
    dropping the source you had over a typo is the worse failure.
    """
    devices = daemon.ask({"cmd": "devices"})
    expect(devices["screens"], "no screens")
    expect(devices["windows"], "no windows with titles")

    monitor = devices["screens"][0]
    chose = daemon.ask({"cmd": "screen", "display": int(monitor["id"])})
    expect(chose.get("reply") == "status", f"choosing a screen answers a status: {chose}")
    expect(
        chose["screen"] == monitor["name"],
        f"chose {monitor['name']!r} but the status says {chose['screen']!r}",
    )

    # A window by part of its title. Every machine running this has a terminal.
    window = daemon.ask({"cmd": "window", "query": "ghostty"})
    expect(window.get("reply") == "status", f"choosing a window answers a status: {window}")
    expect("Ghostty" in window["screen"], f"got {window['screen']!r}")

    missed = daemon.ask({"cmd": "window", "query": "no-such-application-anywhere"})
    expect(missed.get("reply") == "error", f"a miss is an error: {missed}")
    after = daemon.ask({"cmd": "status"})["screen"]
    expect(
        after == window["screen"],
        f"a miss dropped the source: was {window['screen']!r}, now {after!r}",
    )

    absent = daemon.ask({"cmd": "screen", "display": 999})
    expect(absent.get("reply") == "error", f"an absent display is an error: {absent}")
    expect(
        monitor["name"] in absent["message"],
        f"it should say what there is instead: {absent['message']!r}",
    )
    return f"screen -> {chose['screen']}, window -> {window['screen']}, misses change nothing"


# ---- 3 and 4. the camera and the microphone -------------------------------


@check("devices")
def devices(daemon):
    """A camera and a microphone, by name or by id, from separate lists.

    They are separate lists and that matters: a USB microphone is a
    microphone, and a webcam's microphone is also a microphone. Asking the camera list for "hyperx" has to miss rather than
    quietly hand back something that cannot show a picture.

    The id is the handle, never the position: a device unplugged and plugged
    back in returns at a different index.
    """
    found = daemon.ask({"cmd": "devices"})
    expect(found["cameras"], "no cameras at all")
    expect(found["mics"], "no microphones at all")

    camera = found["cameras"][0]
    by_id = daemon.ask({"cmd": "camera", "device": camera["id"]})
    expect(by_id.get("reply") == "status", f"choosing by id answers a status: {by_id}")
    expect(by_id["camera"] == camera["name"], f"got {by_id['camera']!r}")

    # Off is a real state, different from never having chosen one.
    off = daemon.ask({"cmd": "camera", "device": None})
    expect(off["camera"] is None, f"turning the camera off left {off['camera']!r}")

    mic = found["mics"][0]
    chose = daemon.ask({"cmd": "mic", "device": mic["name"]})
    expect(chose.get("reply") == "status", f"choosing a mic by name: {chose}")
    expect(chose["mic"] == mic["name"], f"got {chose['mic']!r}")

    missing = daemon.ask({"cmd": "mic", "device": "Rode NT-USB-that-is-not-here"})
    expect(missing.get("reply") == "error", f"an absent device is an error: {missing}")
    expect(
        mic["name"] in missing["message"],
        f"it should say what there is: {missing['message']!r}",
    )
    return (
        f"{len(found['cameras'])} cameras, {len(found['mics'])} mics; "
        f"by id -> {camera['name']}, by name -> {mic['name']}"
    )


@check("layers")
def layers(daemon):
    """A real camera and window can coexist, move while running and close on cut."""
    found = daemon.ask({"cmd": "devices"})
    expect(found["cameras"], "no camera for a layered capture")
    expect(found["windows"], "no window for a layered capture")
    expect(found["screens"], "no display for a layered capture")
    # The preceding source check now creates layers too. Start this
    # independent layer check on an empty scene instead of assuming an old
    # parallel capture was left behind.
    for existing in daemon.ask({"cmd": "status"})["layers"]:
        daemon.ask({"cmd": "layer-remove", "id": existing["id"]})
    camera = found["cameras"][0]
    # A real running window, not a synthetic title in the source list.
    window = next((w for w in found["windows"] if "Ghostty" in w["name"]), found["windows"][0])
    add_cam = daemon.ask({"cmd": "layer-camera", "id": "face", "device": camera["id"]})
    expect(add_cam.get("reply") == "status", f"camera overlay did not open: {add_cam.get('message')}")
    shaped = daemon.ask({"cmd": "layer-shape", "id": "face", "shape": "circle"})
    expect(shaped.get("reply") == "status" and shaped["layers"][0]["shape"] == "circle", "camera mask did not read back")
    camera_shot = daemon.ask({"cmd": "shot", "of": "camera"})
    expect(camera_shot.get("reply") == "shot" and len(camera_shot["jpeg"]) > 500, "layer camera has no source preview")
    expect(daemon.ask({"cmd": "status"})["camera_flowing"]["captured"] > 0, "layer camera's capture rate is not reported")
    at = daemon.ask({"cmd": "camera-position", "at": {"x": 200, "y": 100}})
    expect(at.get("reply") == "status" and at["layers"][0]["transform"]["x"] == 200, "single-camera alias did not move its layer")
    reset = daemon.ask({"cmd": "layer-position", "id": "face", "at": None})
    expect(reset.get("reply") == "status" and reset["layers"][0]["transform"]["x"] == 0, "camera position did not reset")
    add_window = daemon.ask({"cmd": "layer-window", "id": "app", "query": window["name"].split(" — ")[0]})
    expect(add_window.get("reply") == "status", f"window overlay did not open: {add_window.get('message')}")
    expect([layer["id"] for layer in add_window["layers"]] == ["face", "app"], "overlays lost their order")
    for layer_id in ("face", "app"):
        preview = daemon.ask({"cmd": "layer-shot", "id": layer_id})
        expect(preview.get("reply") == "shot" and len(preview["jpeg"]) > 500,
               f"no preview for layer {layer_id}: {preview.get('message')}")
    for layer in add_window["layers"]:
        size = layer["source"]
        expect(size["width"] > 0 and size["height"] > 0, f"no native size: {size}")
        expect((layer["transform"]["width"], layer["transform"]["height"]) ==
               (size["width"], size["height"]), f"layer was stretched to a preset: {layer}")
    source = add_window["layers"][1]["source"]
    crop = {"x": 0, "y": 0, "width": min(100, source["width"]), "height": min(100, source["height"])}
    cropped = daemon.ask({"cmd": "layer-crop", "id": "app", "crop": crop})
    expect(cropped.get("reply") == "status" and cropped["layers"][1]["crop"] == crop, "crop did not read back")
    invalid = daemon.ask({"cmd": "layer-crop", "id": "app", "crop": {"x": source["width"], "y": 0, "width": 1, "height": 1}})
    expect(invalid.get("reply") == "error", "an out-of-bounds crop was accepted")
    expect(daemon.ask({"cmd": "layer-crop", "id": "app", "crop": None})["layers"][1]["crop"] is None, "crop off did not restore the source")
    transform = {"x": 100, "y": 200, "width": 800, "height": 450, "degrees": 90}
    moved = daemon.ask({"cmd": "layer-transform", "id": "app", "transform": transform})
    expect(moved.get("reply") == "status" and moved["layers"][1]["transform"] == transform, "transform did not read back")
    reordered = daemon.ask({"cmd": "layer-move", "id": "app", "index": 0})
    expect([layer["id"] for layer in reordered["layers"]] == ["app", "face"], "layer order did not change")
    removed = daemon.ask({"cmd": "layer-remove", "id": "app"})
    expect([layer["id"] for layer in removed["layers"]] == ["face"], "window did not close")
    expect(daemon.ask({"cmd": "layer-shot", "id": "app"}).get("reply") == "error", "removed layer still has a preview")
    display = found["screens"][0]
    added = daemon.ask({"cmd": "layer-screen", "id": "desk", "display": int(display["id"])})
    expect(added.get("reply") == "status", f"display layer did not open: {added.get('message')}")
    expect([layer["id"] for layer in added["layers"]] == ["face", "desk"], "display layer has a reserved order")
    expect(added["layers"][1]["source"]["name"] == display["name"], "display ID selected another screen")
    expect(added["layers"][1]["source"]["width"] > 0, "display layer delivered no pixels")
    replaced = daemon.ask({"cmd": "layer-replace-window", "id": "desk", "query": window["name"].split(" — ")[0]})
    expect(replaced.get("reply") == "status" and [l["id"] for l in replaced["layers"]] == ["face", "desk"],
           f"source change lost layer order: {replaced.get('message')}")
    refused = daemon.ask({"cmd": "layer-replace-window", "id": "desk", "query": "window-that-does-not-exist"})
    expect(refused.get("reply") == "error" and daemon.ask({"cmd": "status"})["layers"][1]["id"] == "desk",
           "an unavailable window changed the running layer")
    restored = daemon.ask({"cmd": "layer-replace-screen", "id": "desk", "display": int(display["id"])})
    expect(restored.get("reply") == "status" and restored["layers"][1]["source"]["kind"] == "screen",
           f"display did not return under its ID: {restored.get('message')}")
    before = restored["layer_flowing"]["desk"]["captured"]
    hidden = daemon.ask({"cmd": "layer-visible", "id": "desk", "on": False})
    expect(hidden.get("reply") == "status" and not hidden["layers"][1]["visible"], "display did not hide")
    expect([l["id"] for l in hidden["layers"]] == ["face", "desk"], "hiding closed or reordered a capture")
    expect(hidden["layer_flowing"]["desk"]["captured"] >= before, "hiding reset capture counters")
    expect(daemon.ask({"cmd": "layer-shot", "id": "desk"}).get("reply") == "shot",
           "hidden capture lost its source preview")
    shown = daemon.ask({"cmd": "layer-visible", "id": "desk", "on": True})
    expect(shown.get("reply") == "status" and shown["layers"][1]["visible"], "display did not return")
    expect(shown["layer_flowing"]["desk"]["captured"] >= before, "showing reopened the capture")
    screen_shot = daemon.ask({"cmd": "shot", "of": "screen"})
    expect(screen_shot.get("reply") == "shot" and len(screen_shot["jpeg"]) > 500, "layer display has no source preview")
    expect(daemon.ask({"cmd": "status"})["flowing"]["captured"] > 0, "layer display's capture rate is not reported")
    expect(daemon.ask({"cmd": "layer-move", "id": "desk", "index": 0})["layers"][0]["id"] == "desk", "display cannot move")
    expect(daemon.ask({"cmd": "layer-remove", "id": "desk"})["layers"][0]["id"] == "face", "display did not close")
    cut = daemon.ask({"cmd": "hide-everything"})
    expect(cut.get("reply") == "status" and not cut["layers"], "panic left a camera running")
    return "camera + window + display, transform and order read back, cut closes captures"


# ---- 5. a capture that runs ------------------------------------------------


@check("capture")
def capture(daemon):
    """A capture starts, delivers, survives a swap, and can be pointed at
    nothing.

    The assertion is one frame, not many, and that is the finding rather than a
    weak test. ScreenCaptureKit speaks only when the picture changes: measured
    on this machine, a busy monitor gave 373 frames in five seconds and an idle
    one gave exactly one and then nothing. Demanding a rate here would fail on
    whichever display happens to be showing a still window, and passing on that
    would be worse than the test not existing.

    What it does prove is that pointing the capture somewhere makes frames
    appear at the size asked for, that swapping mid-capture keeps working
    (this is where a pipeline usually breaks), and that nothing is a state the
    engine can be put into rather than an error.
    """
    screens = daemon.ask({"cmd": "devices"})["screens"]
    expect(screens, "no screens")

    seen = []
    for screen in screens:
        chose = daemon.ask({"cmd": "screen", "display": int(screen["id"])})
        expect(chose.get("reply") == "status", f"{screen['name']}: {chose}")
        layer = next(layer for layer in chose["layers"] if layer["source"]["kind"] == "screen")
        flowing = daemon.until(
            lambda: (lambda f: f if f["frames"] >= 1 else None)(
                daemon.ask({"cmd": "status"})["flowing"]
            ),
            f"a frame from {screen['name']}",
        )
        expect(
            (flowing["width"], flowing["height"])
            == (layer["source"]["width"], layer["source"]["height"]),
            f"{screen['name']} delivered {flowing['width']}x{flowing['height']} "
            "instead of its native layer size",
        )
        seen.append(f"{screen['name']} {flowing['frames']}")

    window = daemon.ask({"cmd": "window", "query": "ghostty"})
    expect(window.get("reply") == "status", f"a window capture: {window}")
    daemon.until(
        lambda: daemon.ask({"cmd": "status"})["flowing"]["frames"] >= 1,
        "a frame from a window",
    )

    blank = daemon.ask({"cmd": "share", "on": False})
    expect(blank["screen"] is None, f"sharing nothing left {blank['screen']!r}")
    return f"{', '.join(seen)}, a window, then nothing"


# ---- 5b. the picture goes out at a steady rate -----------------------------


@check("pacing")
def pacing(daemon):
    """The output rate does not depend on what the screen is doing.

    This is the check that matters most: an engine that publishes the capture
    straight reports 0 fps and 0 kbps on an idle display, because
    ScreenCaptureKit says nothing when nothing changes, and a viewer cannot
    tell that apart from a broken stream.

    So the assertion is on the *output* rate and it is deliberately not on the
    captured rate: the two disagreeing is the whole point. A repeated frame
    costs almost nothing in H264, because there is no difference to encode.
    """
    screens = daemon.ask({"cmd": "devices"})["screens"]
    expect(screens, "no screens")

    import time

    measured = []
    for screen in screens:
        daemon.ask({"cmd": "screen", "display": int(screen["id"])})
        daemon.until(
            lambda: daemon.ask({"cmd": "status"})["flowing"]["frames"] >= 1,
            f"the pacer to start on {screen['name']}",
        )
        began = time.monotonic()
        first = daemon.ask({"cmd": "status"})["flowing"]
        time.sleep(2.0)
        after = daemon.ask({"cmd": "status"})["flowing"]
        elapsed = time.monotonic() - began

        out = (after["frames"] - first["frames"]) / elapsed
        got = after["captured"] - first["captured"]
        # A generous band: 30 is the target, and anything from 25 to 33 is a
        # loaded machine rather than a broken pacer. Outside it means the
        # schedule is drifting, which is what would put audio and picture out
        # of step over a long live.
        expect(
            25 <= out <= 33,
            f"{screen['name']} went out at {out:.0f} fps, not near 30 "
            f"(the capture gave {got} frames in that time)",
        )
        measured.append(f"{screen['name']} {got} in / {out:.0f} fps out")

    # With nothing in the scene there is no picture to count: libobs goes on
    # drawing black, and the motor says so by counting nothing, which is what
    # keeps a plan from going live with an empty scene.
    daemon.ask({"cmd": "share", "on": False})
    time.sleep(0.3)
    idle = daemon.ask({"cmd": "status"})["scene_flowing"]
    time.sleep(1.0)
    still = daemon.ask({"cmd": "status"})["scene_flowing"]
    expect(
        still["frames"] == idle["frames"] == 0,
        f"an empty scene counted {still['frames'] - idle['frames']} frames as a picture",
    )
    measured.append("nothing shared, nothing counted")
    return "; ".join(measured)


# ---- 6. the camera opens and delivers --------------------------------------


@check("camera")
def camera(daemon):
    """Every camera on this machine opens, delivers frames, and lets go.

    Listing a camera needs no permission and opening one does, so this is the
    first check that touches the camera grant. It walks all of them rather than
    the first: they are not the same shape; a webcam at 1280x960, another at
    1920x1080, a built-in that warms up slower than the
    rest, a Desk View at 1920x1440, and a virtual camera from OBS. A capture
    path that only ever saw one of those is a capture path that has not been
    tested.

    Closing is checked too, and it is not a formality: it used to leave the
    frame count and the frame size behind, so the status went on describing a
    camera that had been let go. A status that lies is worse than one that is
    slow.
    """
    cameras = daemon.ask({"cmd": "devices"})["cameras"]
    expect(cameras, "no cameras at all")

    opened = []
    for camera in cameras:
        chose = daemon.ask({"cmd": "camera", "device": camera["id"]})
        expect(
            chose.get("reply") == "status",
            f"{camera['name']} would not open: {chose.get('message', chose)}",
        )
        flowing = daemon.until(
            lambda: (lambda f: f if f["captured"] >= 3 else None)(
                daemon.ask({"cmd": "status"})["camera_flowing"]
            ),
            f"frames from {camera['name']}",
        )
        expect(
            flowing["width"] > 0 and flowing["height"] > 0,
            f"{camera['name']} delivered frames with no size",
        )
        # Held at the picture's rate, never the device's fastest: the HP 430
        # left to a preset runs at 61 a second into a picture drawn at 30.
        # Two seconds is enough to tell sixty from thirty;
        # the floor is low because a webcam in a dim room lets its exposure
        # take the rate down (measured: 20 at night), which is not the
        # engine's to hold.
        before = daemon.ask({"cmd": "status"})["camera_flowing"]
        time.sleep(2)
        after = daemon.ask({"cmd": "status"})["camera_flowing"]
        rate = (after["captured"] - before["captured"]) / 2
        held = after.get("held")
        if held is not None:
            expect(held == OUTPUT_FPS, f"{camera['name']} is held at {held}, not {OUTPUT_FPS}")
            expect(
                5 <= rate <= 36,
                f"{camera['name']} ran at {rate:.0f} a second while held at {held}",
            )
        opened.append(
            f"{camera['name']} {flowing['width']}x{flowing['height']} at {rate:.0f}/s"
            + (f" held at {held}" if held is not None else ", the device's own rate")
        )

    daemon.ask({"cmd": "camera", "device": None})
    after = daemon.ask({"cmd": "status"})
    expect(after["camera"] is None, f"closing left {after['camera']!r} named")
    expect(
        after["camera_flowing"]["captured"] == 0 and after["camera_flowing"]["width"] == 0,
        f"closing left the counters behind: {after['camera_flowing']}",
    )
    return f"{len(opened)} opened: {'; '.join(opened)}"


# ---- 8, 9. text and timers, generated on the picture ----------------------


@check("elements")
def elements(daemon):
    """A text and a timer are pictures of their own: the scene has frames with
    nothing captured, a timer counts only while started, and both read back
    in the scene's order.

    They replaced the cards: the words are a layer the operator places, and
    a countdown is a timer element, started and stopped by its id.
    """
    for existing in daemon.ask({"cmd": "status"})["layers"]:
        daemon.ask({"cmd": "layer-remove", "id": existing["id"]})
    text = {"id": "title", "x": 200, "y": 200, "width": 1000, "height": 160,
            "kind": "text", "text": "Chegando já"}
    timer = {"id": "clock", "x": 700, "y": 450, "width": 520, "height": 160,
             "kind": "timer", "seconds": 10}
    for element in (text, timer):
        added = daemon.ask({"cmd": "scene-element-add", "element": element})
        expect(added.get("reply") == "status", f"adding {element['id']}: {added}")
    daemon.until(
        lambda: daemon.ask({"cmd": "status"})["scene_flowing"]["frames"] >= 1,
        "the picture to start with only elements in it",
    )
    first = daemon.ask({"cmd": "status"})["scene_flowing"]["frames"]
    started = daemon.ask({"cmd": "scene-timer-start", "id": "clock"})
    expect(started.get("reply") == "status", f"starting the timer: {started}")
    time.sleep(1.5)
    later = daemon.ask({"cmd": "status"})["scene_flowing"]["frames"]
    rate = (later - first) / 1.5
    expect(rate >= 20, f"the picture ran at {rate:.0f} fps while the timer counted")
    shot = daemon.ask({"cmd": "layer-shot", "id": "clock"})
    expect(shot.get("reply") == "shot", f"a timer has a picture of its own: {shot}")
    moved = daemon.ask({"cmd": "layer-move", "id": "clock", "index": 0})
    order = next(s for s in moved["scenes"] if s["name"] == moved["active_scene"])["order"]
    expect(order == ["clock", "title"], f"the scene's order did not change: {order}")
    for element in (text, timer):
        daemon.ask({"cmd": "layer-remove", "id": element["id"]})
    return f"text and timer drawn, {rate:.0f} fps out while counting, reordered"


# ---- 11. the panic button --------------------------------------------------


@check("panic")
def panic(daemon):
    """One button: every layer off, microphone muted, and the live goes on.

    It is one button and not four commands because it is pressed in the moment
    somebody walks into the room, and four round trips is four chances for one
    of them to be the one that does not arrive. Muting is part of it and used
    to be missing, which is the worst of both: the picture hidden and the
    microphone still open.
    """
    import time

    screen = daemon.ask({"cmd": "devices"})["screens"][0]
    camera = daemon.ask({"cmd": "devices"})["cameras"][0]
    daemon.ask({"cmd": "screen", "display": int(screen["id"])})
    daemon.ask({"cmd": "camera", "device": camera["id"]})
    daemon.ask({"cmd": "go-live"})
    daemon.until(
        lambda: daemon.ask({"cmd": "status"})["flowing"]["frames"] >= 1,
        "the picture to start",
    )

    hidden = daemon.ask({"cmd": "hide-everything"})
    expect(hidden.get("reply") == "status", f"hiding everything: {hidden}")

    expect(hidden["muted"], "hiding everything must mute you too")
    expect(
        hidden["mic"] is None,
        "and close the microphone: muted is a gain of zero on an open device",
    )
    expect(hidden["music_to_stream"] is False, "and keep the bed off the stream")
    expect(hidden["screen_sound"] is False, "and the screen's sound too")
    expect(not hidden["layers"], f"layers stayed up: {hidden['layers']}")
    expect(hidden["camera"] is None, f"the camera stayed at {hidden['camera']!r}")
    expect(hidden["screen"] is None, f"the screen stayed at {hidden['screen']!r}")
    expect(hidden["on_air"], "it is a break, not the end of the live")
    daemon.ask({"cmd": "stop"})
    return "every layer off, camera and screen closed, muted, still on air"


# ---- the screen's sound: a switch every face reads, off until asked -------


@check("screen-sound")
def screen_sound(daemon):
    """What the Mac plays reaches the audience only when asked. The capture
    delivers it whether or not it is sent, so the switch is a gain in the
    mixer and flipping it mid-live restarts nothing: the picture's frame
    count keeps rising through it.
    """
    import time

    daemon.ask({"cmd": "hide-everything"})
    screen = daemon.ask({"cmd": "devices"})["screens"][0]
    daemon.ask({"cmd": "screen", "display": int(screen["id"])})
    daemon.until(
        lambda: daemon.ask({"cmd": "status"})["flowing"]["frames"] >= 1,
        "the picture to start",
    )
    expect(
        daemon.ask({"cmd": "status"})["screen_sound"] is False,
        "the screen is chosen; its sound is not, until asked",
    )
    before = daemon.ask({"cmd": "status"})["flowing"]["frames"]
    sent = daemon.ask({"cmd": "screen-sound", "on": True})
    expect(sent.get("reply") == "status", f"a switch answers with a status: {sent}")
    expect(sent["screen_sound"], "and reads back on")
    time.sleep(1.0)
    after = daemon.ask({"cmd": "status"})["flowing"]["frames"]
    expect(after - before >= 20, f"the picture ran at {after - before} fps through the switch")

    # The sound itself keeps arriving. ScreenCaptureKit hands over the
    # system's audio continuously, silence included, so with the screen
    # shared the count of samples grows with the clock whether or not
    # anything plays: three seconds are at least two seconds of samples.
    # The first build delivered exactly 248 buffers and then nothing, in
    # every process, because each buffer was retained once more than it was
    # released and the capture's pool of them ran dry.
    heard = daemon.ask({"cmd": "status"})["hearing"]["screen_samples"]
    time.sleep(3.0)
    later = daemon.ask({"cmd": "status"})["hearing"]
    expect(
        later["screen_complaint"] is None,
        f"the screen's sound is refused: {later['screen_complaint']}",
    )
    expect(
        later["screen_samples"] - heard >= 2 * 48_000 * 2,
        f"the screen's sound stopped arriving: {heard} -> {later['screen_samples']} "
        f"samples over three seconds",
    )
    kept = daemon.ask({"cmd": "screen-sound", "on": False})
    expect(not kept["screen_sound"], "and off again")
    daemon.ask({"cmd": "share", "on": False})
    return (
        f"off by default, on and off by asking, {after - before} fps through it, "
        f"{(later['screen_samples'] - heard) // (48_000 * 2)}s of the screen's sound in 3s"
    )


# ---- a filter over the scene ----------------------------------------------


@check("filter")
def scene_filter(daemon):
    """A WGSL filter over the whole scene changes the picture that goes out,
    and off restores it. A file that does not build is refused and leaves the
    filter that was there."""

    def middle():
        """The picture's middle pixel, summed: inside the screen whatever its
        shape, where a corner can be the bars beside a screen that is not
        16:9, which no filter makes anything but black."""
        shot = daemon.ask({"cmd": "shot", "of": "scene"})
        expect(shot.get("reply") == "shot", f"scene shot: {shot}")
        rgb = subprocess.run(
            ["ffmpeg", "-v", "error", "-f", "mjpeg", "-i", "pipe:0",
             "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"],
            input=base64.b64decode(shot["jpeg"]), capture_output=True, check=True,
        ).stdout
        expect(len(rgb) == shot["width"] * shot["height"] * 3, "decoded JPEG size")
        at = (shot["height"] // 2 * shot["width"] + shot["width"] // 2) * 3
        return sum(rgb[at:at + 3])

    screen = daemon.ask({"cmd": "devices"})["screens"][0]
    daemon.ask({"cmd": "screen", "display": int(screen["id"])})
    desk = daemon.ask({"cmd": "status"})["layers"][0]["id"]
    daemon.ask({"cmd": "layer-transform", "id": desk,
                "transform": {"x": 0, "y": 0, "width": 1920, "height": 1080, "degrees": 0}})
    daemon.until(lambda: daemon.ask({"cmd": "status"})["scene_flowing"]["frames"] > 2,
                 "the screen to reach the scene")
    plain = middle()
    path = os.path.abspath("engine/motor-obs/examples/invert.wgsl")
    selected = daemon.ask({"cmd": "shader", "path": path})
    expect(selected.get("reply") == "status" and selected.get("shader") == path,
           f"filter selection: {selected}")
    time.sleep(0.5)
    inverted = middle()
    expect(abs(inverted - (765 - plain)) < 60,
           f"the scene filter did not invert the middle: {plain} -> {inverted}")
    broken = os.path.join(tempfile.gettempdir(), "remux-parity-broken.wgsl")
    with open(broken, "w") as f:
        f.write("@fragment fn main() -> nonsense")
    rejected = daemon.ask({"cmd": "shader", "path": broken})
    expect(rejected.get("reply") == "error", "a filter that does not build was accepted")
    expect(daemon.ask({"cmd": "status"})["shader"] == path, "the refusal replaced the filter")
    daemon.ask({"cmd": "shader", "path": None})
    time.sleep(0.5)
    restored = middle()
    expect(abs(restored - plain) < 36, f"off left the filter on: {plain} -> {inverted} -> {restored}")
    daemon.ask({"cmd": "share", "on": False})
    return f"middle {plain} -> {inverted} -> {restored}; a broken file kept the filter"


# ---- a microphone that leaves and comes back ------------------------------


@check("headset")
def headset(daemon):
    """A Bluetooth headset's voice is read whole, whatever phase it lands on.

    The headset delivers twenty milliseconds a callback, and whether a read
    of the mixer lands while one is still being pushed is the luck of where
    the two clocks met when the microphone opened: one opening in five can
    starve 192 blocks in 45 s, a crackle every twenty milliseconds, while the
    other four read clean on the same build. So one opening proves nothing
    either way. This opens the headset again and again, each time a
    new draw, and counts the blocks read short once the ring has settled.

    Needs a Bluetooth headset connected, which is why it is asked for by name.
    """
    connected = json.loads(
        subprocess.run(
            ["blueutil", "--connected", "--format", "json"],
            capture_output=True, text=True, check=True,
        ).stdout or "[]"
    )
    mics = daemon.ask({"cmd": "devices"})["mics"]
    mic = next(
        (mic for device in connected for mic in mics if mic["name"].startswith(device["name"])),
        None,
    )
    expect(mic, "no connected Bluetooth microphone; connect a headset")

    def hearing():
        return daemon.ask({"cmd": "status"})["hearing"]

    rounds = []
    for _ in range(12):
        chose = daemon.ask({"cmd": "mic", "device": mic["id"]})
        expect(chose.get("reply") == "status", f"{mic['name']} would not open: {chose}")
        daemon.until(lambda: hearing()["samples"] > 0 or None, f"samples from {mic['name']}")
        time.sleep(3.0)  # the ring fills to its target; a headset waking bursts
        before = hearing()["starved"]
        time.sleep(10.0)
        rounds.append(hearing()["starved"] - before)
        daemon.ask({"cmd": "mic", "device": None})
    expect(
        not any(rounds),
        f"blocks read short once settled, per opening: {rounds}: silent samples in the voice",
    )
    return f"{mic['name']}: twelve openings, 10 s each, no block read short"


@check("replug")
def replug(daemon):
    """The chosen microphone drops and returns, and nobody has to touch the
    engine: the device list moves, the status says unplugged while it is
    away, and it delivers again by itself when it is back.

    Needs a Bluetooth headset connected and `blueutil` (brew) to take it away
    and bring it back, which is why it is asked for by name. The night this
    was written the headset dropped mid-live and never delivered again until
    the engine was restarted, and a microphone plugged in while the engine
    ran never reached the list: AVFoundation learns about devices through the
    main run loop, and the engine's main thread was blocked on a channel.
    """
    connected = json.loads(
        subprocess.run(
            ["blueutil", "--connected", "--format", "json"],
            capture_output=True, text=True, check=True,
        ).stdout or "[]"
    )
    mics = daemon.ask({"cmd": "devices"})["mics"]
    pair = next(
        ((device, mic) for device in connected for mic in mics
         if mic["name"].startswith(device["name"])),
        None,
    )
    expect(pair, "no connected Bluetooth microphone to take away; connect a headset")
    device, mic = pair

    def status():
        return daemon.ask({"cmd": "status"})

    generation = status()["devices_generation"]
    chose = daemon.ask({"cmd": "mic", "device": mic["id"]})
    expect(chose.get("reply") == "status", f"{mic['name']} would not open: {chose}")
    daemon.until(
        lambda: status()["hearing"]["samples"] > 0 or None,
        f"samples from {mic['name']}",
    )

    try:
        subprocess.run(["blueutil", "--disconnect", device["address"]], check=True)
        daemon.until(
            lambda: status()["devices_generation"] != generation or None,
            "the device list to notice the headset leaving",
            budget=15.0,
        )
        said = daemon.until(
            lambda: (lambda h: h if h["complaint"] else None)(status()["hearing"]),
            "the microphone to say it is unplugged",
            budget=15.0,
        )
        expect("unplugged" in said["complaint"], f"it said {said['complaint']!r} instead")
    finally:
        subprocess.run(["blueutil", "--connect", device["address"]], check=False)

    back = daemon.until(
        lambda: (lambda h: h if h["samples"] > 0 and h["complaint"] is None else None)(
            status()["hearing"]
        ),
        "the microphone to deliver again after coming back, by itself",
        budget=40.0,
        every=0.5,
    )
    daemon.ask({"cmd": "mic", "device": None})
    return (
        f"{mic['name']} left (list {generation} -> {status()['devices_generation']}), "
        f"said unplugged, and came back delivering by itself at {back['level_db']:.0f} dB"
    )


# ---- 12, 13. the microphone, and the gate on real sound --------------------


@check("mic")
def mic(daemon):
    """Every microphone opens, delivers samples, and reports a real level.

    Reading a microphone makes no sound: this opens an input and never an
    output, which is why it can run at three in the morning.

    The check walks all of them because they are not the same shape, and that
    is not hypothetical. Measured before the format conversion was asked for:
    of four microphones on this machine one spoke 32-bit float and three did
    not, and one of the three is the one actually used. The engine now asks
    CoreAudio for float and lets it convert, rather than carrying three
    decoders and three chances to get a sample wrong.

    The level is measured *before* the gate on purpose. A meter reading the
    gated signal sits at the floor and tells nobody where to put the threshold.
    """
    mics = daemon.ask({"cmd": "devices"})["mics"]
    expect(mics, "no microphones at all")

    heard = []
    for microphone in mics:
        chose = daemon.ask({"cmd": "mic", "device": microphone["id"]})
        expect(
            chose.get("reply") == "status",
            f"{microphone['name']} would not open: {chose.get('message', chose)}",
        )
        listening = daemon.until(
            lambda: (lambda h: h if h["samples"] > 0 else None)(
                daemon.ask({"cmd": "status"})["hearing"]
            ),
            f"samples from {microphone['name']}",
        )
        expect(
            listening["complaint"] is None,
            f"{microphone['name']}: {listening['complaint']}",
        )
        # A level outside the meter's own range means the reading is nonsense,
        # not that the room is loud or quiet.
        expect(
            -60.0 <= listening["level_db"] <= 0.0,
            f"{microphone['name']} read {listening['level_db']} dB, off the meter",
        )
        heard.append(f"{microphone['name'].split()[0]} {listening['level_db']:.0f}dB")

    daemon.ask({"cmd": "mic", "device": None})
    after = daemon.ask({"cmd": "status"})
    expect(after["mic"] is None, f"closing left {after['mic']!r} named")
    expect(
        after["hearing"]["samples"] == 0,
        f"closing left {after['hearing']['samples']} samples behind",
    )
    return f"{len(heard)} opened: {', '.join(heard)}"


# ---- 14, 15, 17. the music, the mixer and the meters -----------------------


@check("music")
def music(daemon):
    """A genre plays, the fader moves the level, skipping never repeats, and
    the bed can be kept off the air while it still plays.

    **Nothing reaches the speakers here.** Starting the music opens them, on
    purpose (hearing the bed is what it is for, and a person always hears it
    while it is on), so this check closes them again straight away: it runs
    in a room with somebody in it. Sending the bed out is the optional part:
    with `stream-music` off the music's own meter goes on moving while the
    mix that leaves sits at the floor, which is what an operator who wants to
    audition a track without the audience hearing it is relying on.

    The level is read from a meter with hold and decay, not from a snapshot,
    and that is not decoration. Measured with a raw ten-millisecond RMS, the
    same track read -13 dB on a kick and -60 in the gap after it, which made
    the fader look broken when it was working perfectly. The hold and the
    decay rate are OBS's.
    """
    import time

    # The shared engine arrives here as the panic button left it, with the
    # bed kept off the air; the rows walking together is a claim about a
    # bed that is sent, so say so rather than inherit it.
    daemon.ask({"cmd": "stream-music", "on": True})
    started = daemon.ask({"cmd": "music", "on": True})
    expect(started.get("reply") == "status", f"starting the music: {started}")
    expect(started["music"], "nothing is playing")
    expect(
        daemon.ask({"cmd": "status"})["monitoring"],
        "starting the music is what opens the speakers, and it did not",
    )
    # And closed again at once, for the room this runs in.
    daemon.ask({"cmd": "monitor", "on": False})

    # Long enough for the meter to settle: it falls at 11.76 dB a second, so a
    # reading taken too soon is still on its way down from the previous one.
    readings = []
    for level in (1.0, 0.5):
        daemon.ask({"cmd": "music-volume", "level": level})
        time.sleep(2.0)
        mixing = daemon.ask({"cmd": "status"})["mixing"]
        readings.append(mixing["music_db"])
        # What you hear is what the live hears: the two rows walk together
        # while the bed is sent. Read in one status, so the track cannot move
        # between them.
        expect(
            abs(mixing["music_db"] - mixing["music_out_db"]) < 1.0,
            f"heard {mixing['music_db']:.0f} dB but sent {mixing['music_out_db']:.0f} dB",
        )

    loud, quiet = readings
    expect(loud > quiet, f"the fader did not lower anything: {loud} then {quiet}")
    # Half the travel of the music's fader, linear in dB from -60 to its
    # ceiling of -18, is 21 dB. The band is wide because the music itself
    # moves and the meter is still falling; anything inside it proves the
    # curve, and the curve's exact shape is a unit test.
    step = loud - quiet
    expect(
        12.0 <= step <= 30.0,
        f"half the fader's travel moved the level by {step:.0f} dB, not about 21",
    )

    playing = daemon.ask({"cmd": "status"})["mixing"]["playing"]
    expect(playing, "the engine says nothing is playing while it plays")

    was = daemon.ask({"cmd": "status"})["music"]
    now = daemon.ask({"cmd": "next-track"})["music"]
    expect(now != was, f"skipping played the same track again: {now}")

    # Kept off the air, still playing. The mix's own meter is the one that
    # cannot lie about it: it is measured on what the encoder takes.
    kept = daemon.ask({"cmd": "stream-music", "on": False})
    expect(kept.get("reply") == "status", f"the switch was refused: {kept}")
    expect(not kept["music_to_stream"], "and reads back off")
    # The mix's bar is a peak meter and falls at 11.76 dB a second, so from
    # a loud bed it takes four seconds to reach the floor: waited for, not
    # slept through.
    state = daemon.until(
        lambda: (lambda st: st if st["mixing"]["level_db"] <= -60 else None)(
            daemon.ask({"cmd": "status"})
        ),
        "the mix that leaves falling to the floor with the bed kept off the air",
    )
    expect(
        state["mixing"]["music_db"] > -60,
        f"with stream-music off the music reads {state['mixing']['music_db']:.0f} dB; "
        "it should still move, it is still playing",
    )
    expect(
        state["mixing"]["music_out_db"] <= -60,
        f"and the bed in the mix that leaves reads {state['mixing']['music_out_db']:.0f} dB; "
        "kept off the air it sits at the floor",
    )
    back = daemon.ask({"cmd": "stream-music", "on": True})
    expect(back["music_to_stream"], "and it comes back")

    off = daemon.ask({"cmd": "music", "on": False})
    expect(off["music"] is None, f"turning it off left {off['music']!r}")

    # The duck is proven by unit test and cannot be proven here without a
    # voice: the gate opens on real sound, and this runs in a quiet room.
    return f"{loud:.0f} dB at full, {quiet:.0f} at half, skip changed the track"


@check("voice")
def voice(daemon):
    """The voice keeps step with the picture on a microphone that runs on its
    own clock.

    Every USB or Bluetooth microphone has a crystal of its own. Measured: a
    webcam's delivers 96 256 samples a second against the
    96 000 the mixer consumes, 0.27% fast. Read a block at a time that is 256
    samples a second piling up in the ring: the voice fell half a second
    behind the picture in three minutes, and then the ring was full and threw
    a sample away for every one that arrived, a crackle through a whole
    recording. `remuxd_domain::drift` reads the ring through a ratio instead,
    and this watches the ring hold its target while the device runs fast.
    """
    mics = daemon.ask({"cmd": "devices"})["mics"]
    expect(mics, "no microphones at all")
    # The webcam's, when it is here: the one measured fast. Else the first.
    chosen = next((m for m in mics if "Webcam" in m["name"]), mics[0])
    chose = daemon.ask({"cmd": "mic", "device": chosen["id"]})
    expect(chose.get("reply") == "status", f"{chosen['name']} would not open: {chose}")
    daemon.until(
        lambda: (lambda h: h if h["samples"] > 0 else None)(daemon.ask({"cmd": "status"})["hearing"]),
        f"samples from {chosen['name']}",
    )
    time.sleep(2.0)  # the ring fills to its target and the ratio settles

    target = 1920  # remuxd_domain::drift::TARGET_FRAMES
    starved_before = daemon.ask({"cmd": "status"})["hearing"]["starved"]
    fills = []
    for _ in range(15):
        time.sleep(1.0)
        fills.append(daemon.ask({"cmd": "status"})["hearing"]["buffered"])
    after = daemon.ask({"cmd": "status"})["hearing"]
    daemon.ask({"cmd": "mic", "device": None})

    low, high = min(fills), max(fills)
    # A block at a time, the fill climbs 256 frames a second on this device:
    # fifteen seconds would carry it 3 800 frames past the target. Through
    # the ratio it stays within a block or two of it, both ways.
    expect(
        all(abs(f - target) < 3 * 480 for f in fills),
        f"the ring wandered from {low} to {high} frames against a target of {target}: "
        f"{fills}",
    )
    expect(after["dropped"] == 0, f"the ring threw {after['dropped']} samples away")
    # Once settled, never short: every starved block is a hole of silent
    # samples in the voice, a crackle.
    starved = after["starved"] - starved_before
    expect(starved == 0, f"the mixer read {starved} blocks short once the ring had settled")
    return (
        f"{chosen['name'].split()[0]}: the ring held {low}..{high} frames around {target} "
        f"over 15 s, nothing dropped, starved {after['starved']} blocks in all"
    )


@check("present", alone=True)
def present(_shared):
    """A panel and its engine leave together.

    The panel says `present` once a second. An engine that has heard it and
    then hears nothing for five seconds stops itself, camera and microphone
    released, whatever door the panel left by. Before this, a panel closed by
    the window's own close or a crash left a daemon holding the camera with
    its light on, which is the one thing a person notices about a process
    that outlived its window.

    On an engine of its own: the one thing this check does is end the engine
    it runs on, and every check after it on the shared one read a broken pipe.
    """
    with Daemon(REMUXD) as daemon:
        expect(daemon.ask({"cmd": "present"}).get("reply") == "ok", "present was refused")
        time.sleep(2.0)
        expect(daemon.proc.poll() is None, "it stopped inside the lease")
        began = time.monotonic()
        daemon.until(
            lambda: daemon.proc.poll() is not None or None,
            "the engine stopping by itself",
            budget=8.0,
            every=0.25,
        )
        took = time.monotonic() - began + 2.0
        expect(daemon.proc.returncode == 0, f"it stopped with {daemon.proc.returncode}")
        return f"said present once, and the engine stopped by itself {took:.1f}s later, exit 0"


# ---- the negative control -------------------------------------------------


# ---- 19, 20, 21. what goes out -------------------------------------------


def probe(path, *entries):
    """Ask ffprobe something, or say what it complained about."""
    got = subprocess.run(
        ["ffprobe", "-v", "error", *entries, "-of", "csv=p=0", path],
        capture_output=True,
        text=True,
    )
    return got.stdout.strip(), got.stderr.strip()


def publish_to_a_file(seconds, into):
    """Run a live whose destination is a file, and hand back the file.

    A file is a destination like any other: the same encoder, the same mixer,
    the same two pipes and the same ffmpeg. What it is not is a relay, which
    `byo` proves with one running. Everything this proves about the
    stream is true of the stream that goes to a relay.
    """
    with Daemon(REMUXD, REMUXD_RTMP=into) as engine:
        screens = engine.ask({"cmd": "devices"})["screens"]
        mics = engine.ask({"cmd": "devices"})["mics"]
        expect(screens, "no screen to publish")
        expect(mics, "no microphone to publish")
        engine.ask({"cmd": "screen", "display": int(screens[0]["id"])})
        engine.ask({"cmd": "mic", "device": mics[0]["id"]})
        engine.until(
            lambda: engine.ask({"cmd": "status"})["flowing"]["frames"] >= 30,
            "a picture to publish",
        )
        engine.until(
            lambda: engine.ask({"cmd": "status"})["hearing"]["samples"] > 0,
            "a microphone to publish",
        )
        live = engine.ask({"cmd": "go-live"})
        expect(live.get("reply") == "ok", f"go live was refused: {live}")
        expect(
            engine.ask({"cmd": "status"})["on_air"],
            "the engine must report the air only once something is going out",
        )
        time.sleep(seconds)
        engine.ask({"cmd": "stop"})
        expect(not engine.ask({"cmd": "status"})["on_air"], "stop must leave the air")
    # ffmpeg is given its chance to write the trailer as the publisher drops.
    time.sleep(1.5)
    return into


@check("encode", alone=True)
def encode(_shared):
    """H264 and AAC, in a container, decoding without a complaint.

    The complaint is the assertion that matters. An earlier build wrote a file
    that ffprobe described perfectly and no decoder could read: the publisher
    was killed rather than stopped, so the last packet was cut in half and the
    stream opened in the middle of a group of pictures. `-f null` is what says
    the difference between a file with the right shape and a file that plays.
    """
    into = os.path.join(tempfile.gettempdir(), "remuxd-smoke-encode.flv")
    if os.path.exists(into):
        os.unlink(into)
    publish_to_a_file(6, into)

    expect(os.path.exists(into), "nothing was written at all")
    size = os.path.getsize(into)
    expect(size > 200_000, f"only {size} bytes came out of six seconds")

    video, _ = probe(into, "-select_streams", "v", "-show_entries",
                     "stream=codec_name,width,height")
    audio, _ = probe(into, "-select_streams", "a", "-show_entries",
                     "stream=codec_name,sample_rate,channels")
    expect(video.startswith("h264,1920,1080"), f"the picture is {video!r}")
    expect(audio.startswith("aac,48000,2"), f"the sound is {audio!r}")

    # Decoded, not remuxed. `-f null` runs a muxer as well as a decoder, and
    # that muxer re-derives its own millisecond timestamps and then complains
    # that two of them match. Measured on a file whose own packets are
    # perfectly monotonic and every frame of which decodes: it still says
    # `non monotonically increasing dts`, about itself. So the decoders are
    # what is read here, and the file's own ordering is asserted separately
    # below, which is the property that warning was gesturing at.
    said = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", into, "-f", "null", "-"],
        capture_output=True, text=True,
    ).stderr
    complained = [
        line for line in said.splitlines()
        if line.startswith("[h264") or line.startswith("[aac") or line.startswith("[flv")
    ]
    expect(not complained, f"it does not decode: {complained[:2]}")

    # The sound has to last as long as the picture. "There is an aac stream"
    # is not that assertion and hid a real defect: a file whose audio stopped
    # after two tenths of a second reports `aac, 48000 Hz, stereo` in every
    # field there is.
    heard, _ = probe(into, "-select_streams", "a", "-show_entries", "packet=pts_time")
    ends = [float(x.rstrip(",")) for x in heard.split() if x.strip(",")]
    expect(ends, "the file carries no audio packets at all")

    stamps, why = probe(into, "-select_streams", "v", "-show_entries", "packet=pts_time")
    expect(stamps, f"no video packets to read: {why}")
    times = [float(x) for x in stamps.split() if x]
    backwards = [i for i in range(1, len(times)) if times[i] <= times[i - 1]]
    expect(
        not backwards,
        f"{len(backwards)} of {len(times)} video packets do not move forward",
    )
    expect(
        ends[-1] > times[-1] - 1.0,
        f"the sound stops at {ends[-1]:.1f}s of a {times[-1]:.1f}s stream",
    )
    return (
        f"{size // 1024} KB, {video} + {audio}, {len(times)} video packets "
        f"in order, everything decodes"
    )


@check("sync", alone=True)
def sync(_shared):
    """Audio and video stay together, against a written budget.

    The budget, agreed before the measurement: **initial offset under 30 ms,
    drift under 50 ms per minute, and no gap between frames larger than three
    times the median.**

    Drift is a *rate*, so it is measured as one. Comparing the last stamp of
    each stream against the first is not that measurement and it lies by a
    factor of five: the tail of a live is whatever was still in the pipes when
    it stopped, and dividing that fixed lump by a short run reported -206, then
    +154, then +22 ms/min for the same engine as the run got longer. What this
    does instead is sample how far apart the two timelines are at points
    through the run and fit a line to it, so a fixed offset lands in the
    intercept where it belongs and only a real difference of rate shows up as
    slope.

    Two faults this check sees and no other kind of test could. A mixer that
    generated its blocks on a software timer and threw
    away its scheduling debt every time it woke late, which cost 206 ms of
    sound a minute. And the two rings, two seconds of picture and two of
    sound, were published as though they were live, which put the sound most of
    a second ahead of the picture for the whole broadcast.
    """
    into = os.path.join(tempfile.gettempdir(), "remuxd-smoke-sync.flv")
    if os.path.exists(into):
        os.unlink(into)
    publish_to_a_file(30, into)

    def stamps(stream):
        raw, why = probe(into, "-select_streams", stream, "-show_entries",
                         "packet=pts_time")
        expect(raw, f"no {stream} packets to read: {why}")
        return [float(x) for x in raw.split() if x]

    video, audio = stamps("v"), stamps("a")
    span = video[-1] - video[0]
    expect(span > 20, f"only {span:.0f}s came out of a thirty second live")

    # How far apart the two are, each second, from the second the live settles
    # to the second before it stops. Both ends are left out on purpose: the
    # first is the pipes filling and the last is them emptying.
    def nearest(stamps_of, moment):
        return min(stamps_of, key=lambda stamp: abs(stamp - moment))

    moments = [float(t) for t in range(2, int(span) - 1)]
    apart = [(t, (nearest(audio, t) - nearest(video, t)) * 1000) for t in moments]
    expect(len(apart) > 10, "too few samples to fit a rate to")

    mean_t = sum(t for t, _ in apart) / len(apart)
    mean_d = sum(d for _, d in apart) / len(apart)
    above = sum((t - mean_t) * (d - mean_d) for t, d in apart)
    below = sum((t - mean_t) ** 2 for t, _ in apart)
    drift = (above / below) * 60 if below else 0.0
    worst = max(abs(d) for _, d in apart)

    gaps = [video[i + 1] - video[i] for i in range(len(video) - 1)]
    middle = statistics.median(gaps)
    offset = abs(video[0] - audio[0]) * 1000

    said = (
        f"over {span:.0f}s: offset {offset:.0f}ms, drift {drift:+.0f}ms/min, "
        f"never more than {worst:.0f}ms apart, gap {middle * 1000:.0f}ms median "
        f"and {max(gaps) * 1000:.0f}ms worst, {len(video) / span:.2f} fps"
    )
    expect(offset < 30, f"the sound does not start with the picture. {said}")
    expect(
        max(gaps) < middle * 3,
        f"the picture stalled longer than three frames. {said}",
    )
    expect(abs(drift) < 50, f"the sound and the picture come apart. {said}")
    # A rate of zero is not enough on its own: two streams can hold a constant
    # 300 ms apart forever and drift will report nothing wrong.
    expect(worst < 50, f"the sound sits too far from the picture. {said}")
    return said


#: The relay on this machine. A destination that pushes anywhere else is a
#: real platform.
LAB = {"localhost", "127.0.0.1", "mediamtx"}


def real_destinations(rows):
    """The hosts, among these `(ingest_url, armed, sandbox)` rows, that a
    publish would put on a real audience: a real platform, armed, and not in
    its sandbox."""
    hosts = []
    for url, armed, sandbox in rows:
        host = urllib.parse.urlparse(url).hostname
        if host not in LAB and armed and not sandbox:
            hosts.append(host)
    return hosts


def lab_only():
    """Refuse, before anything is published or armed, a destination that
    would go live for real.

    The lab's relay is always fine; a real platform is fine only in its
    sandbox (Twitch's bandwidth test, an unlisted YouTube broadcast). A real,
    armed, non-sandbox destination is never a smoke's. Only hosts are
    named here, never a key. The rows are the destinations file the daemon
    reads (`REMUX_DESTINATIONS`, else the operator's own).
    """
    path = os.environ.get("REMUX_DESTINATIONS") or os.path.expanduser("~/.config/remux/destinations.json")
    try:
        with open(path) as kept:
            rows = [(k["url"], bool(k.get("armed")), bool(k.get("sandbox"))) for k in json.load(kept)]
    except (OSError, ValueError):
        rows = []
    real = real_destinations(rows)
    expect(
        not real,
        f"a real destination is armed outside its sandbox ({', '.join(sorted(set(real)))}): "
        "a smoke never goes live for real; `remux sandbox <id> on` first",
    )


# ---- the free flow: no account, the CLI, a platform that is only a door ----

REMUX = "engine/target/release/remux"
#: The daemon under test: the core's (libobs) unless REMUXD_BIN names another binary.
REMUXD = os.environ.get("REMUXD_BIN", "engine/target/release/remuxd")
LAB_PLATFORM = "http://127.0.0.1:9998"
LAB_RELAY = "http://127.0.0.1:9996"


def lab(api, path):
    """One path on a lab mediamtx (no auth), or `None` when it is not there."""
    try:
        with urllib.request.urlopen(f"{api}/v3/paths/get/{path}", timeout=3) as answer:
            return json.load(answer)
    except Exception:
        return None


def lab_up():
    try:
        urllib.request.urlopen(f"{LAB_PLATFORM}/v3/paths/list", timeout=3)
        urllib.request.urlopen(f"{LAB_RELAY}/v3/paths/list", timeout=3)
    except Exception:
        raise Failed("the lab is not up: make smoke.lab.up")


def cli(engine, *words, env=None):
    """The CLI against this daemon, with this daemon's own files."""
    said = subprocess.run(
        [REMUX, *words], env={**engine.env, **(env or {})}, capture_output=True, text=True
    )
    return said.returncode, (said.stdout + said.stderr).strip()


def keyfile(name, key):
    path = os.path.join(tempfile.gettempdir(), f"remux-smoke-{os.getpid()}-{name}.key")
    with open(path, "w") as f:
        os.chmod(path, 0o600)
        f.write(key + "\n")
    return path


def scene_ready(engine):
    screens = engine.ask({"cmd": "devices"})["screens"]
    mics = engine.ask({"cmd": "devices"})["mics"]
    expect(screens and mics, "no screen or microphone")
    engine.ask({"cmd": "screen", "display": int(screens[0]["id"])})
    engine.ask({"cmd": "mic", "device": mics[0]["id"]})
    engine.until(
        lambda: engine.ask({"cmd": "status"})["flowing"]["frames"] >= 30, "a picture"
    )
    engine.until(
        lambda: engine.ask({"cmd": "status"})["hearing"]["samples"] > 0, "the microphone"
    )


def go_live(engine, name):
    code, plan = cli(engine, "plan", "--json")
    expect(code == 0, f"plan: {plan}")
    fingerprint = json.loads(plan)["fingerprint"]
    code, said = cli(engine, "live", "--confirm", str(fingerprint))
    expect(code == 0, f"live: {said}")
    code, said = cli(engine, "wait", "on-air", "--for", "20")
    expect(code == 0, f"wait on-air: {said}")
    code, said = cli(engine, "wait", "live", name, "--for", "30")
    expect(code == 0, f"wait live {name}: {said}")


def received(api, path, budget=20):
    """The lab door calls the path ready and the bytes climb: a stream a decoder
    can read, not a connection that sent nothing."""
    deadline = time.monotonic() + budget
    while time.monotonic() < deadline:
        state = lab(api, path) or {}
        if state.get("ready") and state.get("bytesReceived", 0) > 200_000:
            return state["bytesReceived"]
        time.sleep(0.25)
    raise Failed(f"{api} never had {path} ready with bytes (last {lab(api, path)})")


@check("free", alone=True)
def free(_shared):
    """A person with no account: the CLI, a destination typed in with its key,
    a recording and a live at once, straight to the platform's door.

    No login, no relay, no chat: `remux chat` says so. The platform is the
    lab's, so the key is fake and the live is nobody's.
    """
    lab_only()
    lab_up()
    films = os.path.join(tempfile.gettempdir(), f"remux-smoke-free-{os.getpid()}")
    with Daemon(REMUXD, REMUXD_RECORD_DIR=films) as engine:
        # two destinations, the two ways a key comes in: a file, and stdin
        code, said = cli(
            engine, "destination", "add", "custom", "fake",
            "--url", "rtmp://127.0.0.1:1937/live", "--key-file", keyfile("free", "free-key"),
        )
        expect(code == 0 and "fake" in said, f"destination add: {said}")
        typed = subprocess.run(
            [REMUX, "destination", "add", "twitch", "tw", "--url", "rtmp://127.0.0.1:1937/live", "--key", "-"],
            input="tw-key\n", env=engine.env, capture_output=True, text=True,
        )
        expect(typed.returncode == 0, f"destination add from stdin: {typed.stdout}{typed.stderr}")
        # the sandbox is the rule for a real platform; the lab's door takes the
        # bandwidth-test query and ignores it
        code, said = cli(engine, "sandbox", "2", "on")
        expect(code == 0, f"sandbox: {said}")
        code, said = cli(engine, "chat")
        expect("no chat wire" in said, f"chat without a wire: {said}")
        scene_ready(engine)
        code, said = cli(engine, "health")
        expect(code == 0 and "can go live" in said, f"health: {said}")
        # arming is the file's, and the plan reads it back
        code, said = cli(engine, "disarm", "2")
        expect(code == 0, f"disarm: {said}")
        code, plan = cli(engine, "plan", "--json")
        armed = [d["name"] for d in json.loads(plan)["destinations"] if d["armed"]]
        expect(armed == ["fake"], f"the plan after disarm: {armed}")
        code, said = cli(engine, "arm", "2")
        expect(code == 0, f"arm: {said}")
        code, said = cli(engine, "record", "start")
        expect(code == 0, f"record start: {said}")
        code, said = cli(engine, "music", "lofi")
        expect(code == 0, f"music: {said}")
        go_live(engine, "fake")
        code, said = cli(engine, "wait", "live", "tw", "--for", "30")
        expect(code == 0, f"wait live tw: {said}")
        got = received(LAB_PLATFORM, "live/free-key")
        got_tw = received(LAB_PLATFORM, "live/tw-key")
        engine.until(
            lambda: engine.ask({"cmd": "status"})["mixing"]["playing"]
            and engine.ask({"cmd": "status"})["mixing"]["music_db"] > -60,
            "the music bed to be heard in the mix", budget=10,
        )
        code, said = cli(engine, "stop")
        expect(code == 0, f"stop: {said}")
        code, said = cli(engine, "record", "stop")
        expect(code == 0, f"record stop: {said}")
        code, said = cli(engine, "history", "--json")
        lives = json.loads(said)
        expect(code == 0 and len(lives) == 1, f"history: {said}")
    time.sleep(1.5)
    made = glob.glob(os.path.join(films, "*.mp4"))
    expect(len(made) == 1, f"one recording, got {made}")
    shape, why = probe(made[0], "-select_streams", "v", "-show_entries", "stream=codec_name,width,height")
    expect(shape.startswith("h264,1920,1080"), f"the picture is {shape!r} ({why})")
    sound, _ = probe(made[0], "-select_streams", "a", "-show_entries", "stream=codec_name")
    expect(sound.startswith("aac"), f"the sound is {sound!r}")
    return f"the platform read {got // 1024} + {got_tw // 1024} KB straight from the engine, two doors; one mp4 beside it"


@check("byo", alone=True)
def byo(_shared):
    """A person with no account and servers of their own: a relay built as
    docs/relay.md says, and a chat bridge speaking docs/wire.md.

    The live goes to the relay, the relay's egress to the platform; the chat
    comes down the bridge, a delete goes back up it, and the bridge going
    away and coming back is seen and survived.
    """
    lab_only()
    lab_up()
    wire = "engine/target/debug/examples/wire"
    expect(os.path.exists(wire), "cargo build -p remuxd --example wire first")
    log = open(os.path.join(tempfile.gettempdir(), f"remux-smoke-wire-{os.getpid()}.log"), "w+")
    bridge = subprocess.Popen([wire], stderr=log, stdout=subprocess.DEVNULL)
    try:
        config = os.path.join(tempfile.gettempdir(), f"remux-smoke-byo-{os.getpid()}.config.toml")
        env = {"REMUX_CONFIG": config}
        # the shell keeps the wire before the daemon starts, as a person would
        code, said = subprocess.run(
            [REMUX, "chat", "--url", "ws://127.0.0.1:9999"],
            env={**os.environ, **env}, capture_output=True, text=True,
        ).returncode, ""
        expect(code == 0, "chat --url")
        with Daemon(REMUXD, **env) as engine:
            code, said = cli(
                engine, "destination", "add", "custom", "relay",
                "--url", "rtmp://127.0.0.1:1938", "--key-file", keyfile("byo", "scene"),
            )
            expect(code == 0, f"destination add: {said}")
            engine.until(
                lambda: engine.ask({"cmd": "chat", "since": 0})["reachable"], "the wire to come up"
            )
            engine.until(
                lambda: engine.ask({"cmd": "chat", "since": 0})["lines"], "a line down the wire"
            )
            code, said = cli(engine, "chat", "--json")
            lines = json.loads(said)["lines"]
            expect(lines and lines[0]["from"] == "wire 9999", f"chat: {said[:120]}")
            # pushed, not polled: one connection, replies as lines arrive
            follow = subprocess.Popen(
                [REMUX, "chat", "-f", "--json"], env=engine.env, stdout=subprocess.PIPE, text=True
            )
            time.sleep(3.5)
            follow.kill()
            pushed = [l for l in follow.stdout.read().splitlines() if l.startswith('{"reply":"chat"')]
            expect(len(pushed) >= 2, f"chat -f pushed {len(pushed)} replies in 3 s")
            # a delete leaves every face here and goes up the wire by the id
            code, said = cli(engine, "delete", str(lines[0]["seq"]))
            expect(code == 0, f"delete: {said}")
            engine.until(
                lambda: all(l["seq"] != lines[0]["seq"] for l in engine.ask({"cmd": "chat", "since": 0})["lines"]),
                "the deleted line to be gone here",
            )
            def bridge_got_delete():
                log.seek(0)
                return f'"delete":{{"id":"{lines[0]["id"]}"' in log.read()
            engine.until(bridge_got_delete, "the bridge to receive the delete", budget=5)

            scene_ready(engine)
            go_live(engine, "relay")
            on_relay = received(LAB_RELAY, "scene")
            on_platform = received(LAB_PLATFORM, "live/relayed", budget=30)
            code, said = cli(engine, "stop")
            expect(code == 0, f"stop: {said}")

            # the bridge goes away and comes back; the engine says so and recovers
            bridge.kill()
            engine.until(
                lambda: not engine.ask({"cmd": "chat", "since": 0})["reachable"], "the wire to be seen down", budget=6
            )
            bridge = subprocess.Popen([wire], stderr=log, stdout=subprocess.DEVNULL)
            engine.until(
                lambda: engine.ask({"cmd": "chat", "since": 0})["reachable"], "the wire to come back", budget=8
            )

            # the bridge is swapped for another while the engine runs: no restart
            other = subprocess.Popen([wire, "9989"], stderr=log, stdout=subprocess.DEVNULL)
            try:
                code, said = cli(engine, "chat", "--url", "ws://127.0.0.1:9989")
                expect(code == 0 and "opening" in said, f"chat --url while running: {said}")
                engine.until(
                    lambda: any(
                        l["from"] == "wire 9989" for l in engine.ask({"cmd": "chat", "since": 0})["lines"]
                    ),
                    "a line from the other bridge",
                    budget=8,
                )
            finally:
                other.kill()
        return f"relay read {on_relay // 1024} KB, the platform {on_platform // 1024} KB through the egress; chat pushed, deleted, reconnected, swapped live"
    finally:
        bridge.kill()
        log.close()


@check("record", alone=True)
def record(_shared):
    """One MP4 on disk, playable, across a monitor swap.

    `avc1` is the assertion, not a detail. MP4 cannot carry a source whose
    resolution changes, which is why Chromium invented the `avc3` tag, and
    `avc3` is unplayable on macOS because AVFoundation reads the parameter sets
    out of `stsd` and only `avc1` puts them there. The compositor is one
    surface of one size that never changes, so the swap happens behind it and
    the file never notices, which is the whole reason that surface exists.

    Recording needs no relay and no destination, so this asks for neither.
    """
    films = os.path.join(tempfile.gettempdir(), "remuxd-smoke-films")
    for old in glob.glob(os.path.join(films, "*.mp4")):
        os.unlink(old)

    with Daemon(REMUXD, REMUXD_RECORD_DIR=films) as engine:
        screens = engine.ask({"cmd": "devices"})["screens"]
        mics = engine.ask({"cmd": "devices"})["mics"]
        expect(screens and mics, "no screen or microphone to record")
        engine.ask({"cmd": "screen", "display": int(screens[0]["id"])})
        engine.ask({"cmd": "mic", "device": mics[0]["id"]})
        engine.until(
            lambda: engine.ask({"cmd": "status"})["flowing"]["frames"] >= 30,
            "a picture to record",
        )
        started = engine.ask({"cmd": "record-start"})
        expect(started.get("reply") == "ok", f"recording was refused: {started}")
        expect(engine.ask({"cmd": "status"})["recording"], "the clock must be running")
        time.sleep(5)

        swapped = len(screens) > 1
        if swapped:
            engine.ask({"cmd": "screen", "display": int(screens[1]["id"])})
            time.sleep(4)
        else:
            # One monitor here, so the source is changed the other way the
            # product allows: the screen's layer hides and shows again.
            desk = engine.ask({"cmd": "status"})["layers"][0]["id"]
            engine.ask({"cmd": "layer-visible", "id": desk, "on": False})
            time.sleep(2)
            engine.ask({"cmd": "layer-visible", "id": desk, "on": True})
            time.sleep(2)

        engine.ask({"cmd": "record-stop"})
        expect(not engine.ask({"cmd": "status"})["recording"], "the clock must stop")
    time.sleep(1.5)

    made = glob.glob(os.path.join(films, "*.mp4"))
    expect(len(made) == 1, f"a session is one file, got {len(made)}: {made}")
    film = made[0]

    shape, why = probe(film, "-select_streams", "v", "-show_entries",
                       "stream=codec_name,codec_tag_string,width,height")
    expect(shape.startswith("h264,avc1,1920,1080"), f"the picture is {shape!r} ({why})")
    sound, _ = probe(film, "-select_streams", "a", "-show_entries",
                     "stream=codec_name,sample_rate,channels")
    expect(sound.startswith("aac,48000,2"), f"the sound is {sound!r}")

    length, _ = probe(film, "-show_entries", "format=duration")
    seconds = float(length or 0)
    expect(
        seconds > 8,
        f"the swap cut the file short: {seconds:.1f}s of a nine second session",
    )

    # And the sound has to last as long as the picture. Asserting that the
    # stream exists is not the same assertion and hid a real defect: a
    # recording whose audio stopped after two tenths of a second still reports
    # `aac, 48000 Hz, stereo` in every field ffprobe shows for the stream.
    heard, _ = probe(film, "-select_streams", "a", "-show_entries", "packet=pts_time")
    ends = [float(x.rstrip(",")) for x in heard.split() if x.strip(",")]
    expect(ends, "the file carries no audio packets at all")
    expect(
        ends[-1] > seconds - 1.0,
        f"the sound stops at {ends[-1]:.1f}s of a {seconds:.1f}s recording",
    )

    complained = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", film, "-f", "null", "-"],
        capture_output=True, text=True,
    ).stderr
    broken = [
        line for line in complained.splitlines()
        if line.startswith("[h264") or line.startswith("[aac") or line.startswith("[mov")
    ]
    expect(not broken, f"it does not decode: {broken[:2]}")

    return (
        f"{os.path.getsize(film) // 1024} KB, {shape}, {seconds:.1f}s continuous "
        f"across {'a monitor swap' if swapped else 'the screen going and coming back'}"
    )


# ---- 16. hearing your own mix ---------------------------------------------


def rings(name):
    """The three sequence counters in the preview region, read as the panel
    reads them: `shm_open` and `mmap`, through libc, because Python has no
    POSIX shared memory of its own on macOS."""
    import ctypes
    import mmap

    libc = ctypes.CDLL(None, use_errno=True)
    libc.shm_open.restype = ctypes.c_int
    fd = libc.shm_open(name.encode(), 0, 0)
    expect(fd >= 0, f"cannot open the preview region {name!r}")
    try:
        with mmap.mmap(fd, 64, prot=mmap.PROT_READ) as head:
            return tuple(int.from_bytes(head[at : at + 8], "little") for at in (8, 16, 24))
    finally:
        import os

        os.close(fd)


@check("watching")
def watching(daemon):
    """The preview is published while a face keeps saying it is watching,
    and stops a few seconds after the last time one did.

    A lease, not a count. The count lived in the engine's memory, so an engine
    restarted under a live panel started at zero and never published a frame
    again: the panel showed the last picture it had, screen, camera and scene
    all still, and a card put up never appeared. Nothing caught it because
    every check here starts a fresh engine and a fresh face together. This
    one reads the region the panel reads and watches the counters, so the
    three states are told apart: renewed, lapsed, and back.
    """
    import time

    screens = daemon.ask({"cmd": "devices"})["screens"]
    expect(screens, "no screen to compose")
    daemon.ask({"cmd": "screen", "display": int(screens[0]["id"])})
    region = daemon.ask({"cmd": "status"})["preview"]
    expect(region, "this engine has no preview region")
    name = region["name"]

    daemon.ask({"cmd": "watching", "on": True})
    time.sleep(1.0)
    first = rings(name)
    time.sleep(2.0)
    second = rings(name)
    expect(second[0] > first[0], f"watched, the scene ring did not move: {first} -> {second}")
    # At the pace of the picture, not a slideshow of it: the compositor draws
    # thirty a second whatever the screen does, and a self-view published at
    # fifteen was the camera that "stuttered slightly" in the panel.
    pace = (second[0] - first[0]) / 2.0
    expect(pace >= 25, f"the scene ring moves {pace:.0f} a second; the picture is 30")

    # Four seconds without a renewal is past the three-second lease.
    time.sleep(4.0)
    lapsed = rings(name)
    time.sleep(1.0)
    still = rings(name)
    expect(
        still[0] == lapsed[0],
        f"nobody renewed and the scene ring kept moving: {lapsed} -> {still}",
    )

    daemon.ask({"cmd": "watching", "on": True})
    time.sleep(1.0)
    back = rings(name)
    expect(back[0] > still[0], f"watched again, the ring did not come back: {still} -> {back}")
    return f"scene ring {first[0]}->{second[0]} watched, {lapsed[0]}->{still[0]} lapsed, {back[0]} back"


@check("monitor", alone=True)
def monitor(_shared):
    """The speakers, and the voice that must not be on them.

    **This check is designed to make no sound**, which is why it can run
    unattended at all: with the music off, what the monitor bus carries is
    silence, so the output device opens, buffers go round, and the room stays
    quiet. What it proves in that state is the property that matters and the
    one nobody can hear anyway: **the microphone is not in it**. On speakers a
    voice on this bus comes back through the microphone and goes out twice.

    So: open a microphone, watch it hear the room, and watch the monitor stay
    at the floor while it does. If the voice ever leaked in, this is where it
    would show, silently, and long before somebody discovered it live.

    What it cannot prove is that the music actually reaches the speakers, and
    that is the one line in this file that needs a person: turn the music on,
    turn this on, and hear it. Everything up to that is here.
    """
    with Daemon(REMUXD) as engine:
        mics = engine.ask({"cmd": "devices"})["mics"]
        expect(mics, "no microphone to keep out of the monitor")
        engine.ask({"cmd": "mic", "device": mics[0]["id"]})
        engine.ask({"cmd": "music", "on": False})
        engine.until(
            lambda: engine.ask({"cmd": "status"})["hearing"]["samples"] > 0,
            "the microphone to be delivering",
        )

        off = engine.ask({"cmd": "status"})["mixing"]["monitor_db"]
        expect(
            off <= -60,
            f"nothing is playing, so the monitor is at the floor, not {off}",
        )

        # And now it is on, with nothing to play, which is silence in the room.
        on = engine.ask({"cmd": "monitor", "on": True})
        expect(on.get("reply") == "status", f"the switch was refused: {on}")
        expect(on["monitoring"], "and reads back on")

        # A couple of seconds of the microphone hearing the room, while the
        # bus is watched.
        loudest = -120.0
        heard = -120.0
        for _ in range(20):
            state = engine.ask({"cmd": "status"})
            loudest = max(loudest, state["mixing"]["monitor_db"])
            heard = max(heard, state["hearing"]["level_db"])
            time.sleep(0.1)

        engine.ask({"cmd": "monitor", "on": False})
        expect(
            not engine.ask({"cmd": "status"})["monitoring"],
            "and it goes off again",
        )

    expect(
        loudest <= -60,
        f"the voice is on the speakers: the monitor reached {loudest:.1f} dB with "
        f"nothing playing, while the microphone heard {heard:.1f} dB",
    )
    return (
        f"the output opened and stayed silent with nothing playing "
        f"({loudest:.0f} dB) while the microphone heard {heard:.0f} dB. "
        f"That the music reaches the speakers is the one thing only a person can say"
    )


@check("loopback", alone=True)
def loopback(_shared):
    """The music really does come out of the speakers, and the witness is the
    microphone.

    **This one makes a sound in the room**, which is why it is not in the
    default run: `make remuxd.smoke S=loopback`, with somebody there.

    It is the half `monitor` cannot reach. The microphone hearing the speakers
    is the exact feedback path the design warns about and the reason the voice
    is kept off that bus, so it is also the only instrument in the building
    that can answer the question without a person saying "I heard it".

    Measured with the system volume at 13 of 100: the room sat at -60 dB with the music playing and the monitor off, and rose to
    -43.6 dB when the monitor went on. Sixteen decibels is not ambiguous.
    """
    with Daemon(REMUXD, REMUX_MUSIC_DIR="music") as engine:
        mics = engine.ask({"cmd": "devices"})["mics"]
        expect(mics, "no microphone to hear the speakers with")
        # The built-in one, which is the only microphone certain to be in the
        # same room as the speakers: a headset would hear nothing and would be
        # a pass that means the opposite of what it says.
        chosen = next((m for m in mics if "MacBook" in m["name"]), mics[0])
        engine.ask({"cmd": "mic", "device": chosen["id"]})
        engine.ask({"cmd": "genre", "name": "lofi"})
        engine.ask({"cmd": "music-volume", "level": 1.0})
        time.sleep(3)

        def loudest(seconds):
            heard = -120.0
            for _ in range(int(seconds * 10)):
                heard = max(heard, engine.ask({"cmd": "status"})["hearing"]["level_db"])
                time.sleep(0.1)
            return heard

        before = loudest(4)
        # Whatever happens between here and the end of this block, the room
        # goes quiet again. Killing the daemon would stop the sound anyway,
        # because the audio queue dies with the process; this is so that an
        # interrupt in the middle leaves the *engine* quiet too, and not just
        # this run of it.
        engine.ask({"cmd": "monitor", "on": True})
        try:
            time.sleep(1.5)
            after = loudest(6)
        finally:
            engine.ask({"cmd": "monitor", "on": False})
            engine.ask({"cmd": "music", "on": False})

    rose = after - before
    expect(
        rose > 4,
        f"the microphone heard nothing new: {before:.1f} dB with the monitor off, "
        f"{after:.1f} with it on. Either no sound left the speakers, or the "
        f"system volume is at zero, or the microphone is not in the room",
    )
    return (
        f"the room went from {before:.0f} dB to {after:.0f} dB when the monitor "
        f"went on, heard by the {chosen['name']}"
    )


# ---- the cost of the picture ---------------------------------------------


def cpu_seconds(pid):
    """Cumulative CPU time of a process, in seconds.

    `ps -o %cpu` on macOS is a decaying average over up to a minute, which is
    the wrong instrument for a five second window: it carries the startup burst
    into every sample. Cumulative time differenced over a known wall clock is
    exact and carries nothing.
    """
    # procps prints cputime in whole seconds; the kernel's own counters do
    # not round. Where /proc is, read it.
    try:
        with open(f"/proc/{pid}/stat") as stat:
            fields = stat.read().rsplit(")", 1)[1].split()
        ticks = os.sysconf("SC_CLK_TCK")
        return (int(fields[11]) + int(fields[12])) / ticks
    except (OSError, IndexError, ValueError):
        pass
    raw = subprocess.run(
        ["ps", "-o", "cputime=", "-p", str(pid)], capture_output=True, text=True
    ).stdout.strip()
    if not raw:
        raise Failed(f"process {pid} is gone")
    minutes, seconds = raw.rsplit(":", 1)
    hours = 0
    if ":" in minutes:
        hours, minutes = minutes.split(":")
    return int(hours) * 3600 + int(minutes) * 60 + float(seconds)


def rss_mb(pid):
    raw = subprocess.run(
        ["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True
    ).stdout.strip()
    return int(raw) / 1024 if raw else 0.0


def cost(daemon, seconds=12.0, settle=2.0):
    """What this daemon costs over a window, and what it produced in it.

    The settle is not politeness: opening a capture allocates buffers and
    negotiates a format, and a window that starts on the first frame reads the
    startup burst as the running cost. Measured both ways on the same camera:
    16.0% with a one second settle, 28.2% with none.

    It runs first among the checks that have the machine to themselves, and
    that is not politeness either: after `channel`, which publishes a whole
    live, the same engine read 14.1% against the 13.7% it reads on a cold one,
    and the figure it is compared against is 13.9%.

    Twelve seconds, not six: a six second window reads 13.6%, 13.8% and 14.1%
    for the same engine, which is a coin toss rather than a measurement.

    The camera is chosen by name rather than by position, because the device
    list does not come back in a stable order and two cameras do not cost the
    same: picking whichever was first moved the figure by four
    points between runs. It prefers the built-in one, which is the camera every
    machine has and the fair subject for a comparison; a USB webcam costs more
    and measuring one would be measuring the webcam.

    **It still wants a quiet machine.** With the development stack up, which
    `smoke byo` needs, the same engine read 13.7% and then 17.2% twelve
    seconds apart: the lab's relay and platform are on the same
    cores. Run this one on its own, and read a failure as "measure
    it again with `make dev.down`" before reading it as a regression.

    The frames come back with the percentage on purpose. A cheap number is only
    interesting beside the work it did: a daemon that composed nothing would
    post the best figure on this page.
    """
    time.sleep(settle)
    began_cpu = cpu_seconds(daemon.proc.pid)
    began_wall = time.monotonic()
    began_frames = daemon.ask({"cmd": "status"})["flowing"]["frames"]
    time.sleep(seconds)
    took = time.monotonic() - began_wall
    used = cpu_seconds(daemon.proc.pid) - began_cpu
    frames = daemon.ask({"cmd": "status"})["flowing"]["frames"] - began_frames
    return used / took * 100, rss_mb(daemon.proc.pid), frames / took


@check("headto", alone=True)
def headto(_shared):
    """The cost of the picture, measured, against a budget.

    Its own daemon, and a release one: the shared daemon of this suite is a
    debug build, and a debug build's cost is a number about rustc rather than
    about this design.

    Four states, because the interesting figures are differences and a total
    alone names nothing. A hidden layer keeps its capture running and is not
    composited, which is what splits the cost of having a camera from the
    cost of drawing it:

        C - D  is the camera's capture, alone
        (B - A) - (C - D)  is one more layer through the compositor

    The budget is 13.9% of a core for the screen and a camera together, which
    is what a browser-based engine costs capturing, compositing, encoding and
    publishing; every number below stops before the encoder, so the comparison
    is unkind on purpose.
    """
    with Daemon(REMUXD) as engine:
        screens = engine.ask({"cmd": "devices"})["screens"]
        cameras = engine.ask({"cmd": "devices"})["cameras"]
        expect(screens, "no screens to measure against")
        expect(cameras, "no camera to measure against")

        engine.ask({"cmd": "screen", "display": int(screens[0]["id"])})
        engine.until(
            lambda: engine.ask({"cmd": "status"})["flowing"]["frames"] >= 30,
            "the picture to be running before its cost is read",
        )
        screen_cpu, screen_mb, screen_fps = cost(engine)

        # By name, and always the same one. The device list does not come back
        # in a stable order, so `cameras[0]` measured a different camera on
        # every run and the figure moved by four points between them. A
        # benchmark that picks its own subject is not one.
        chosen = next(
            (camera for camera in cameras if "MacBook Pro Camera" in camera["name"]),
            sorted(cameras, key=lambda camera: camera["name"])[0],
        )
        engine.ask({"cmd": "camera", "device": chosen["id"]})
        engine.until(
            lambda: engine.ask({"cmd": "status"})["camera_flowing"]["frames"] >= 15,
            "the camera to be delivering before its cost is read",
        )
        both_cpu, both_mb, both_fps = cost(engine)

        for layer in engine.ask({"cmd": "status"})["layers"]:
            engine.ask({"cmd": "layer-visible", "id": layer["id"], "on": False})
        behind_cpu, _, _ = cost(engine)

        engine.ask({"cmd": "camera", "device": None})
        card_cpu, _, _ = cost(engine)

    capture = behind_cpu - card_cpu
    layer = (both_cpu - screen_cpu) - capture
    # One memory figure, for the whole run, and named as what it is. Resident
    # memory is a high-water mark: the state that did the least work reported
    # the most, because it ran last and kept what the states before it had
    # touched. Reporting it per state was reporting the order they ran in.
    said = (
        f"screen {screen_cpu:.1f}% {screen_fps:.0f}fps, "
        f"+camera {both_cpu:.1f}% {both_fps:.0f}fps "
        f"(of which capture {capture:.1f}%, one more layer {layer:.1f}%), "
        f"{max(screen_mb, both_mb):.0f}MB at its highest, on {chosen['name']}"
    )

    # The rate first. A cheap engine that dropped to 20 fps would pass a CPU
    # budget by failing at the job, and an idle screen is exactly where a
    # browser-based engine reports zero.
    expect(screen_fps > 25, f"the picture must hold its rate. {said}")
    expect(both_fps > 25, f"a camera must not cost the rate. {said}")

    # The budget is a browser-based engine doing strictly more than this
    # measures. There is no honest way to call this met while it is over.
    expect(both_cpu < 13.9, f"this must cost less than the budget, 13.9%. {said}")
    return said


@check("negative")
def negative(daemon):
    """Prove the suite can fail, and that a refusal is still an answer.

    Media and device assertions rot into vacuity more than any other kind: a
    list is non-empty, a name is a string, everything passes and nothing is
    being checked. This aims the same machinery at input that is known bad and
    insists it complains.

    It used to also name a verb the engine had not built yet, and to expect
    `unsupported` from it. That verb was `next-track` until the music landed
    and `arm` until the app did, and there is no next one: every command in the
    protocol is answered, `Reply::Unsupported` is deleted, and a reply nothing
    can produce is one somebody eventually writes a reader for. What replaced
    it is the property that made deleting it safe.
    """
    try:
        expect(False, "this must be reported")
    except Failed:
        pass
    else:
        raise Failed("expect() did not raise: every check above proves nothing")

    refused = daemon.ask({"cmd": "not-a-verb"})
    expect(refused.get("reply") == "error", f"a bad verb must be refused, got {refused}")

    # A verb the engine understands and cannot carry out is a sentence, not a
    # closed socket. `arm` on a destination nobody kept is exactly that.
    cannot = daemon.ask({"cmd": "arm", "adapter": 1, "on": True})
    expect(
        cannot.get("reply") == "error" and "no destination" in cannot.get("message", ""),
        f"a refusal must say which thing is missing, got {cannot}",
    )
    # And it is still serving afterwards.
    expect(daemon.ask({"cmd": "status"}).get("reply") == "status", "it must still answer")

    # The guard that keeps the suite off the real platforms, against a real
    # one and against the lab.
    expect(
        real_destinations([("rtmp://live.twitch.tv/app", True, False), ("rtmp://localhost:1935/live", True, False)])
        == ["live.twitch.tv"],
        "a real platform, armed, outside its sandbox must be refused",
    )
    expect(
        real_destinations([("rtmp://live.twitch.tv/app", True, True), ("rtmp://a.rtmp.youtube.com/live2", False, False)]) == [],
        "a sandbox, or an unarmed one, passes; so does the lab",
    )

    return (
        "the suite can fail, a bad verb is refused, a refusal names what is missing, "
        "a real platform is told from the lab"
    )


def main():
    wanted = sys.argv[1] if len(sys.argv) > 1 else None
    # `loopback` plays music out of the speakers. It is a real check and it is
    # asked for by name, never run by somebody who walked away from the
    # machine.
    running = {
        k: v
        for k, v in CHECKS.items()
        if (k == wanted) or (not wanted and k not in NOISY)
    }
    if not running:
        print(f"no such check: {wanted}. have: {', '.join(CHECKS)}")
        return 1

    failures = 0
    order = list(running)

    def run(name, daemon):
        began = time.monotonic()
        try:
            said = CHECKS[name](daemon)
            # the seconds beside every check: the slow ones are the ones to move
            print(f"  ok    {name:<12} {time.monotonic() - began:5.1f}s  {said}")
            return 0
        except Failed as e:
            print(f"  FAIL  {name:<12} {e}")
        except Exception as e:  # a check that crashes is a failed check
            print(f"  ERROR {name:<12} {type(e).__name__}: {e}")
        return 1

    # The shared daemon has somewhere to send a live, because going live is
    # now something that either works or is refused, and half the checks are
    # about what happens while a live is running. It writes to a file nobody
    # reads: what those checks are about is the engine's state, not the file.
    nowhere = os.path.join(tempfile.gettempdir(), "remuxd-smoke-nowhere.flv")
    shared = [name for name in order if name not in ALONE]
    if shared:
        with Daemon(REMUXD_RTMP=nowhere) as daemon:
            for name in shared:
                failures += run(name, daemon)
    # The benchmark first among these, on the coldest machine this run will
    # see. It sits within a fifth of a point of the figure it is compared
    # against, and running it after a check that published a whole live read
    # 14.1% for an engine that reads 13.7% on its own. A benchmark that goes
    # last is a benchmark of whatever went before it.
    alone = [n for n in order if n in ALONE]
    for name in sorted(alone, key=lambda n: n != "headto"):
        failures += run(name, None)

    print()
    print(f"{len(running) - failures}/{len(running)} passed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
