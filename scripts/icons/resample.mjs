/**
 * 图标母版的解码与缩放，只依赖 Node 内置 zlib，和 encode.mjs 一样不引第三方图像库。
 *
 * - decodePNG：支持 8 位 RGBA、非隔行的 PNG（人物母版就是这种格式）。
 * - resize：可分离 Lanczos3，在预乘 alpha 上插值，透明边缘不会被拉出黑边。
 *   大比例缩小时核按缩放比放宽，等价于先低通再采样，避免锯齿与摩尔纹。
 * - 小尺寸再做一次轻微反锐化，补回 Lanczos 低通带来的发虚。
 */
import zlib from "node:zlib"

export function decodePNG(buf) {
  if (buf.readUInt32BE(0) !== 0x89504e47) throw new Error("不是 PNG 文件")
  let offset = 8
  let width = 0
  let height = 0
  const idat = []
  while (offset < buf.length) {
    const length = buf.readUInt32BE(offset)
    const type = buf.toString("latin1", offset + 4, offset + 8)
    const data = buf.subarray(offset + 8, offset + 8 + length)
    if (type === "IHDR") {
      width = data.readUInt32BE(0)
      height = data.readUInt32BE(4)
      const [depth, color, , , interlace] = data.subarray(8, 13)
      if (depth !== 8 || color !== 6 || interlace !== 0) throw new Error("母版须为 8 位 RGBA、非隔行 PNG")
    } else if (type === "IDAT") idat.push(data)
    else if (type === "IEND") break
    offset += 12 + length
  }
  const raw = zlib.inflateSync(Buffer.concat(idat))
  const stride = width * 4
  const out = Buffer.alloc(stride * height)
  for (let y = 0; y < height; y += 1) {
    const filter = raw[y * (stride + 1)]
    const line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1))
    const row = y * stride
    for (let x = 0; x < stride; x += 1) {
      const a = x >= 4 ? out[row + x - 4] : 0
      const b = y > 0 ? out[row - stride + x] : 0
      const c = x >= 4 && y > 0 ? out[row - stride + x - 4] : 0
      let value = line[x]
      if (filter === 1) value += a
      else if (filter === 2) value += b
      else if (filter === 3) value += (a + b) >> 1
      else if (filter === 4) {
        const p = a + b - c
        const pa = Math.abs(p - a)
        const pb = Math.abs(p - b)
        const pc = Math.abs(p - c)
        value += pa <= pb && pa <= pc ? a : pb <= pc ? b : c
      }
      out[row + x] = value & 0xff
    }
  }
  return { width, height, rgba: out }
}

const lanczos = (x) => {
  if (x === 0) return 1
  if (Math.abs(x) >= 3) return 0
  const px = Math.PI * x
  return (3 * Math.sin(px) * Math.sin(px / 3)) / (px * px)
}

/** 一维权重表：目标第 i 个像素取源像素 [start, start + weights.length) 的加权和 */
function weights(src, dst) {
  const scale = src / dst
  const support = 3 * Math.max(scale, 1)
  const table = []
  for (let i = 0; i < dst; i += 1) {
    const center = (i + 0.5) * scale
    const start = Math.max(0, Math.floor(center - support))
    const end = Math.min(src, Math.ceil(center + support))
    const w = []
    let sum = 0
    for (let j = start; j < end; j += 1) {
      const v = lanczos((j + 0.5 - center) / Math.max(scale, 1))
      w.push(v)
      sum += v
    }
    table.push({ start, w: w.map((v) => v / sum) })
  }
  return table
}

/** 预乘浮点缓冲：[r*a, g*a, b*a, a]，a 取 0–1 */
function premultiply({ width, height, rgba }) {
  const out = new Float64Array(width * height * 4)
  for (let i = 0; i < width * height; i += 1) {
    const a = rgba[i * 4 + 3] / 255
    out[i * 4] = rgba[i * 4] * a
    out[i * 4 + 1] = rgba[i * 4 + 1] * a
    out[i * 4 + 2] = rgba[i * 4 + 2] * a
    out[i * 4 + 3] = a
  }
  return out
}

function resizeFloat(src, sw, sh, dw, dh) {
  const wx = weights(sw, dw)
  const wy = weights(sh, dh)
  const tmp = new Float64Array(dw * sh * 4)
  for (let y = 0; y < sh; y += 1) {
    for (let x = 0; x < dw; x += 1) {
      const { start, w } = wx[x]
      for (let c = 0; c < 4; c += 1) {
        let acc = 0
        for (let k = 0; k < w.length; k += 1) acc += src[(y * sw + start + k) * 4 + c] * w[k]
        tmp[(y * dw + x) * 4 + c] = acc
      }
    }
  }
  const out = new Float64Array(dw * dh * 4)
  for (let x = 0; x < dw; x += 1) {
    for (let y = 0; y < dh; y += 1) {
      const { start, w } = wy[y]
      for (let c = 0; c < 4; c += 1) {
        let acc = 0
        for (let k = 0; k < w.length; k += 1) acc += tmp[((start + k) * dw + x) * 4 + c] * w[k]
        out[(y * dw + x) * 4 + c] = acc
      }
    }
  }
  return out
}

/** 3×3 高斯模糊做反锐化掩模，只作用于预乘颜色与 alpha，不改变轮廓位置 */
function sharpen(buf, size, amount) {
  const kernel = [1, 2, 1, 2, 4, 2, 1, 2, 1]
  const out = new Float64Array(buf.length)
  for (let y = 0; y < size; y += 1) {
    for (let x = 0; x < size; x += 1) {
      for (let c = 0; c < 4; c += 1) {
        let blur = 0
        let k = 0
        for (let dy = -1; dy <= 1; dy += 1) {
          for (let dx = -1; dx <= 1; dx += 1) {
            const sx = Math.min(size - 1, Math.max(0, x + dx))
            const sy = Math.min(size - 1, Math.max(0, y + dy))
            blur += buf[(sy * size + sx) * 4 + c] * kernel[k]
            k += 1
          }
        }
        const i = (y * size + x) * 4 + c
        out[i] = buf[i] + (buf[i] - blur / 16) * amount
      }
    }
  }
  return out
}

/** 小尺寸的锐化强度：越小越需要补回细节，64px 起不再处理 */
export function sharpenAmount(size) {
  if (size <= 24) return 0.45
  if (size <= 48) return 0.3
  return 0
}

/** 缩放为 size×size 的直色 RGBA（Buffer），供 encodePNG 与托盘底图使用 */
export function resize(image, size) {
  if (image.width !== image.height) throw new Error("图标母版必须是正方形")
  let buf = resizeFloat(premultiply(image), image.width, image.height, size, size)
  const amount = sharpenAmount(size)
  if (amount > 0) buf = sharpen(buf, size, amount)
  const out = Buffer.alloc(size * size * 4)
  for (let i = 0; i < size * size; i += 1) {
    const a = Math.min(1, Math.max(0, buf[i * 4 + 3]))
    out[i * 4 + 3] = Math.round(a * 255)
    if (a > 0) {
      for (let c = 0; c < 3; c += 1) out[i * 4 + c] = Math.round(Math.min(255, Math.max(0, buf[i * 4 + c] / a)))
    }
  }
  return out
}
