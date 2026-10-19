// 图片上传（plan 第 12 节）：申请预签名地址 → 浏览器直传存储桶（带进度）→ 确认 → 等 worker 转成 WebP。
import { ApiError, ERROR_MESSAGES, type Schemas } from "@akari/api-client";
import { api, unwrap } from "./browser-api";

export type UploadView = Schemas["UploadView"];
export type ImageVariant = Schemas["ImageVariant"];

/** 与后端 `forum_media::MAX_UPLOAD_BYTES` 一致。 */
export const MAX_IMAGE_BYTES = 10 * 1024 * 1024;
/** 后端 `Format::from_mime` 接受的类型。 */
const ACCEPTED: Record<string, string> = {
  "image/jpeg": "image/jpeg",
  "image/png": "image/png",
  "image/gif": "image/gif",
  "image/webp": "image/webp",
  "image/heic": "image/heic",
  "image/heif": "image/heif",
};
/** 文件选择框的 accept：HEIC 在多数非苹果系统上没有 MIME 类型，补上扩展名。 */
export const IMAGE_ACCEPT = [...Object.keys(ACCEPTED), ".heic", ".heif"].join(",");

/** 编辑器里插入的尺寸：最长边 ≤ 1280 的最大一张（原图小时只有更小的）。 */
const INSERT_MAX_EDGE = 1280;
const POLL_MS = 500;
const PROCESS_TIMEOUT_MS = 60_000;

const FAILURE: Record<string, string> = {
  unsupported: ERROR_MESSAGES.UNSUPPORTED_IMAGE,
  too_large: ERROR_MESSAGES.IMAGE_TOO_LARGE,
  corrupt: "图片已损坏，无法处理",
  missing: "上传没有完成，请重试",
};

/** 上传时声明的类型；不支持的返回 null。 */
export function imageType(file: { name: string; type: string }): string | null {
  if (ACCEPTED[file.type]) return file.type;
  if (!file.type && /\.(heic|heif)$/i.test(file.name)) return "image/heic";
  return null;
}

/** 选好文件后立即检查（后端还会再查文件头与大小）；没问题返回 null。 */
export function imageProblem(file: { name: string; type: string; size: number }): string | null {
  if (!imageType(file)) return ERROR_MESSAGES.UNSUPPORTED_IMAGE;
  if (file.size === 0) return "图片是空文件";
  if (file.size > MAX_IMAGE_BYTES) return ERROR_MESSAGES.IMAGE_TOO_LARGE;
  return null;
}

/** 插入正文用的那一张（从小到大排列，取最长边不超过 1280 的最大一张，都超过时取最小的）。 */
export function pickVariant(variants: readonly ImageVariant[]): ImageVariant | null {
  let best: ImageVariant | null = null;
  for (const v of variants) {
    if (Math.max(v.width, v.height) <= INSERT_MAX_EDGE) best = v;
  }
  return best ?? variants[0] ?? null;
}

/** 处理失败的原因 → 中文。 */
export function failureMessage(error: string | null | undefined): string {
  return (error && FAILURE[error]) || "图片处理失败，请换一张试试";
}

/** PUT 到存储桶：fetch 拿不到上传进度，用 XHR。 */
function putFile(
  url: string,
  headers: Record<string, string>,
  file: Blob,
  onProgress: (fraction: number) => void,
  signal: AbortSignal,
): Promise<void> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("PUT", url);
    for (const [k, v] of Object.entries(headers)) xhr.setRequestHeader(k, v);
    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable) onProgress(e.loaded / e.total);
    };
    xhr.onload = () =>
      xhr.status >= 200 && xhr.status < 300
        ? resolve()
        : reject(new ApiError("NETWORK", "图片上传失败，请重试", xhr.status));
    xhr.onerror = () => reject(new ApiError("NETWORK", "图片上传失败，请检查网络后重试", 0));
    xhr.onabort = () => reject(new DOMException("aborted", "AbortError"));
    signal.addEventListener("abort", () => xhr.abort(), { once: true });
    xhr.send(file);
  });
}

function sleep(ms: number, signal: AbortSignal) {
  return new Promise<void>((resolve, reject) => {
    const t = setTimeout(resolve, ms);
    signal.addEventListener(
      "abort",
      () => {
        clearTimeout(t);
        reject(new DOMException("aborted", "AbortError"));
      },
      { once: true },
    );
  });
}

export interface UploadProgress {
  /** uploading：直传中（带进度 0–1）；processing：等服务器转 WebP */
  phase: "uploading" | "processing";
  fraction: number;
}

/** 完整的上传流程；成功返回处理好的结果（含 WebP 各尺寸），失败抛出带中文提示的错误。 */
export async function uploadImage(
  file: File,
  onProgress: (p: UploadProgress) => void,
  signal: AbortSignal,
): Promise<UploadView> {
  const contentType = imageType(file);
  const problem = imageProblem(file);
  if (problem || !contentType) throw new ApiError("INVALID_PARAMETER", problem ?? "", 400);
  const created = await unwrap(
    api.POST("/api/v1/uploads", { body: { content_type: contentType, size: file.size } }),
  );
  onProgress({ phase: "uploading", fraction: 0 });
  await putFile(
    created.upload_url,
    created.headers,
    file,
    (fraction) => onProgress({ phase: "uploading", fraction }),
    signal,
  );
  onProgress({ phase: "processing", fraction: 1 });
  const path = { params: { path: { id: created.id } } };
  let view = await unwrap(api.POST("/api/v1/uploads/{id}/complete", path));
  const deadline = Date.now() + PROCESS_TIMEOUT_MS;
  while (view.status === "pending" || view.status === "processing") {
    if (Date.now() > deadline) throw new ApiError("INTERNAL", "图片处理超时，请稍后重试", 0);
    await sleep(POLL_MS, signal);
    view = await unwrap(api.GET("/api/v1/uploads/{id}", path));
  }
  if (view.status === "failed") {
    throw new ApiError("INVALID_PARAMETER", failureMessage(view.error), 400);
  }
  return view;
}
