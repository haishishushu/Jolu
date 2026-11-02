"""
把任意一首曲子剪短或延长到宣传片时长，保留原曲的开头和自然结尾。

做法：在曲子中段搜索两处听感最接近的位置（比较前后几秒的频谱），按波形互相关对齐到采样级，
接缝用短交叉淡化。
- 原曲偏长：从前一处直接接到后一处，跳过中间一段；开头 MIN_SPLICE 秒以内不动，保护前奏和起势。
- 原曲偏短：播到后一处时跳回前一处，把这一段循环若干遍（每段 8–20 秒），前奏与结尾仍是原曲的。

用法：python scripts/promo-video/fit-music.py <输入音乐> <输出.wav> [时长秒，默认 76]
"""
import subprocess
import sys

import numpy as np
from scipy.io import wavfile

SR = 44100
HOP = 1024
MIN_SPLICE = 30.0  # 接缝不早于这个时间，保护前奏和第一次起势
TAIL_GUARD = 8.0  # 接缝之后至少还要留这么长再到结尾
XFADE = 0.08


def ffmpeg_exe():
    try:
        import imageio_ffmpeg

        return imageio_ffmpeg.get_ffmpeg_exe()
    except ImportError:
        return "ffmpeg"


def decode(path):
    raw = subprocess.run(
        [ffmpeg_exe(), "-loglevel", "error", "-i", path, "-ac", "2", "-ar", str(SR), "-f", "f32le", "-"],
        capture_output=True,
        check=True,
    ).stdout
    return np.frombuffer(raw, dtype=np.float32).reshape(-1, 2).astype(np.float64)


def spectrogram(mono):
    """按 HOP 分帧的对数频带能量（40 个对数间隔频带），用来比较听感"""
    win = np.hanning(2048)
    frames = (len(mono) - 2048) // HOP
    idx = np.arange(2048)[None, :] + HOP * np.arange(frames)[:, None]
    mag = np.abs(np.fft.rfft(mono[idx] * win, axis=1))
    freqs = np.fft.rfftfreq(2048, 1 / SR)
    edges = np.geomspace(40, 16000, 41)
    bands = np.stack([mag[:, (freqs >= lo) & (freqs < hi)].sum(axis=1) for lo, hi in zip(edges[:-1], edges[1:])], axis=1)
    spec = np.log(bands + 1e-6)
    spec -= spec.mean(axis=1, keepdims=True)
    return spec / (np.linalg.norm(spec, axis=1, keepdims=True) + 1e-9)


def natural_end(audio):
    """最后一个 0.1 秒响度高于 -45 dBFS 的位置"""
    step = int(0.1 * SR)
    mono = audio.mean(axis=1)
    rms = np.sqrt(np.add.reduceat(mono[: len(mono) // step * step] ** 2, np.arange(0, len(mono) // step * step, step)) / step)
    loud = np.nonzero(20 * np.log10(rms + 1e-12) > -45)[0]
    return (loud[-1] + 1) * step / SR


def find_splice(audio, skip, slack=2.5, min_splice=MIN_SPLICE, tail_guard=TAIL_GUARD):
    """在 [skip-slack, skip+slack] 的跳跃量里找听感最接近的一对位置 (a, a+d)，单位秒"""
    spec = spectrogram(audio.mean(axis=1))
    fps = SR / HOP
    before, after = int(2 * fps), int(4 * fps)
    end = natural_end(audio)
    best = (-np.inf, None, None)
    for d in range(int((skip - slack) * fps), int((skip + slack) * fps) + 1):
        a_lo = int(min_splice * fps)
        a_hi = int((end - d / fps - tail_guard) * fps)
        if a_hi <= a_lo:
            continue
        sim = np.einsum("ij,ij->i", spec[:-d], spec[d:])  # 每帧的相似度
        csum = np.concatenate([[0], np.cumsum(sim)])
        a = np.arange(a_lo, min(a_hi, len(sim) - after))
        score = (csum[a + after] - csum[a - before]) / (before + after)
        k = int(np.argmax(score))
        if score[k] > best[0]:
            best = (score[k], a[k] / fps, d / fps)
    if best[1] is None:
        raise SystemExit("曲子太短，找不到合适的接缝")
    return best


def refine(audio, a, d):
    """在 ±25ms 内按波形互相关微调跳跃量，让接缝两侧相位对齐"""
    mono = audio.mean(axis=1)
    i, w, r = int(a * SR), int(0.05 * SR), int(0.025 * SR)
    ref = mono[i - w : i + w]
    j0 = int((a + d) * SR)
    corr = [np.dot(ref, mono[j0 + s - w : j0 + s + w]) for s in range(-r, r + 1)]
    return i, j0 + int(np.argmax(corr)) - r


def stitch(audio, joints, end):
    """joints 为 [(播到第 i 个采样, 跳到第 j 个采样)]，依次拼接并在每个接缝做交叉淡化"""
    x = int(XFADE * SR)
    ramp = np.linspace(0, 1, x)[:, None]
    parts, pos = [], 0
    for i, j in joints:
        parts += [audio[pos : i - x], audio[i - x : i] * (1 - ramp) + audio[j - x : j] * ramp]
        pos = j
    parts.append(audio[pos:end])
    return np.concatenate(parts)


def fit(audio, duration):
    end = natural_end(audio)
    target = int(duration * SR)
    if abs(end - duration) < 0.5:
        out, note = audio[:target], "原曲时长与目标一致，未剪辑"
    elif end > duration:
        score, a, d = find_splice(audio, end - duration)
        i, j = refine(audio, a, d)
        out = stitch(audio, [(i, j)], len(audio))
        note = f"接缝 {i / SR:.2f}s → 原曲 {j / SR:.2f}s（跳过 {(j - i) / SR:.2f}s，相似度 {score:.3f}），原曲自然结尾落在 {end - (j - i) / SR:.2f}s"
    else:
        extra = duration - end
        loops = int(np.ceil(extra / 20))
        if extra / loops < 8:
            raise SystemExit(f"需要延长 {extra:.1f}s，太短，无法凑成 8 秒以上的循环段")
        score, a, d = find_splice(audio, extra / loops, slack=0.6, min_splice=0.2 * end, tail_guard=3.0)
        i, j = refine(audio, a, d)  # 循环段为原曲 [i, j)，播到 j 跳回 i
        out = stitch(audio, [(j, i)] * loops, len(audio))
        note = f"原曲 {i / SR:.3f}s–{j / SR:.3f}s（{(j - i) / SR:.3f}s，相似度 {score:.3f}）循环 {loops} 遍，原曲自然结尾落在 {end + loops * (j - i) / SR:.2f}s"
    out = out[:target]
    if len(out) < target:
        out = np.concatenate([out, np.zeros((target - len(out), 2))])
    # 原曲结尾若还没完全收住，最后 1.5 秒补一个淡出
    fade = int(1.5 * SR)
    out[-fade:] *= np.linspace(1, 0, fade)[:, None]
    return out, note


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    if len(sys.argv) < 3:
        raise SystemExit(__doc__)
    duration = float(sys.argv[3]) if len(sys.argv) > 3 else 76.0
    audio = decode(sys.argv[1])
    out, note = fit(audio, duration)
    peak = np.abs(out).max()
    if peak > 0:
        out *= 0.7 / peak  # 峰值约 -3 dB，给 AAC 编码的过冲留余量
    wavfile.write(sys.argv[2], SR, (out * 32767).astype(np.int16))
    print(f"配乐已适配到 {duration:.2f} 秒：{note}")
