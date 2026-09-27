"""The music folder, file by file: every one is a single MPEG audio stream
and nothing else. It decodes whole, carries no second stream, no blob in its
tags (a picture, a GEOB, a PRIV), and its size matches its length at its
bitrate. Run over what a download brought before it is dropped into the
folder the engine plays from, and over the whole folder after.

    make music.check            # the whole folder
    python3 engine/remuxd/smoke/library.py <dir>   # a staging folder
"""
import json, os, struct, subprocess, sys
OK_FRAMES = {"TIT2","TPE1","TALB","TYER","TDRC","TRCK","TCON","TSSE","TENC","COMM","TXXX","TPE2","TCOM","TPOS","TLEN","TDEN","TDTG","TDOR","TIT1","TIT3","TPUB","TCOP","TBPM","TKEY","TLAN","TMED","TOFN","TOPE","TOAL","TORY","USLT","TSRC","TSOP","TSOT","TSOA","TCMP",
             # chapter frames: DAW markers ("Intro", "Tempo: 120.0") carried over from the WAV's cue chunk
             "CTOC","CHAP",
             # the date Logic Pro writes into the WAV (ID3v2.3 TDAT)
             "TDAT"}
def id3(path):
    with open(path, "rb") as f: head = f.read(10)
    if head[:3] != b"ID3": return 0, []
    ver = head[3]; size = 0
    for b in head[6:10]: size = (size << 7) | (b & 0x7f)
    with open(path, "rb") as f: f.seek(10); body = f.read(size)
    frames, i = [], 0
    while i + 10 <= len(body):
        fid = body[i:i+4]
        if fid == b"\x00\x00\x00\x00": break
        if ver == 4:
            fs = 0
            for b in body[i+4:i+8]: fs = (fs << 7) | (b & 0x7f)
        else: fs = struct.unpack(">I", body[i+4:i+8])[0]
        frames.append((fid.decode("latin1"), fs)); i += 10 + fs
    return size, frames
def probe(path):
    bad = []
    magic = subprocess.run(["file", "-b", path], capture_output=True, text=True).stdout.strip()
    if not (magic.startswith("Audio file with ID3") or magic.startswith("MPEG ADTS, layer III")): bad.append(f"magic: {magic[:60]}")
    p = subprocess.run(["ffprobe","-v","error","-show_streams","-show_format","-of","json",path], capture_output=True, text=True)
    if p.returncode != 0: return [f"ffprobe: {p.stderr.strip()[:80]}"]
    info = json.loads(p.stdout); streams = info["streams"]; fmt = info["format"]
    if len(streams) != 1: bad.append(f"{len(streams)} streams: {[s.get('codec_type')+'/'+s.get('codec_name','?') for s in streams]}")
    elif streams[0]["codec_type"] != "audio" or streams[0]["codec_name"] != "mp3": bad.append(f"stream {streams[0]['codec_type']}/{streams[0]['codec_name']}")
    if fmt.get("format_name") != "mp3": bad.append(f"container {fmt.get('format_name')}")
    dur = float(fmt.get("duration", 0)); size = int(fmt.get("size", 0)); br = int(fmt.get("bit_rate", 0))
    if not 30 <= dur <= 900: bad.append(f"duration {dur:.0f}s")
    expected = dur * br / 8
    if expected and abs(size - expected) / expected > 0.10: bad.append(f"size {size} vs {expected:.0f} from {br//1000}k x {dur:.0f}s")
    tag_size, frames = id3(path)
    if tag_size > 64 * 1024: bad.append(f"id3 tag {tag_size} bytes")
    odd = [f for f, _ in frames if f not in OK_FRAMES]
    if odd: bad.append(f"tag frames {odd}")
    d = subprocess.run(["ffmpeg","-v","error","-xerror","-i",path,"-f","null","-"], capture_output=True, text=True)
    if d.returncode != 0 or d.stderr.strip(): bad.append(f"decode: {d.stderr.strip()[:80] or 'exit '+str(d.returncode)}")
    return bad
roots = sys.argv[1:]
files = sorted(os.path.join(r, g, f) for r in roots for g in sorted(os.listdir(r)) if os.path.isdir(os.path.join(r, g)) for f in os.listdir(os.path.join(r, g)))
failed = 0
for path in files:
    bad = probe(path)
    if bad: failed += 1; print(f"  BAD  {path}: {'; '.join(bad)}")
print(f"{len(files) - failed}/{len(files)} files are one clean MPEG audio stream, decoded whole, plain tags, size matching length")
sys.exit(1 if failed else 0)
