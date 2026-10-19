import { describe, expect, it } from "vitest";
import { failureMessage, imageProblem, imageType, MAX_IMAGE_BYTES, pickVariant } from "./uploads";

describe("image checks before uploading", () => {
  it("accepts the formats the server converts, including extension-only HEIC", () => {
    expect(imageType({ name: "a.png", type: "image/png" })).toBe("image/png");
    expect(imageType({ name: "IMG_1.HEIC", type: "" })).toBe("image/heic");
    expect(imageType({ name: "a.svg", type: "image/svg+xml" })).toBeNull();
    expect(imageType({ name: "a.heic.exe", type: "" })).toBeNull();
  });

  it("rejects wrong types, empty and oversized files", () => {
    expect(imageProblem({ name: "a.png", type: "image/png", size: 10 })).toBeNull();
    expect(imageProblem({ name: "a.bmp", type: "image/bmp", size: 10 })).toMatch(/只支持/);
    expect(imageProblem({ name: "a.png", type: "image/png", size: 0 })).not.toBeNull();
    expect(imageProblem({ name: "a.png", type: "image/png", size: MAX_IMAGE_BYTES + 1 })).toMatch(
      /10 MB/,
    );
    expect(imageProblem({ name: "a.png", type: "image/png", size: MAX_IMAGE_BYTES })).toBeNull();
  });
});

describe("pickVariant", () => {
  const v = (w: number, h: number) => ({ width: w, height: h, url: `${w}x${h}.webp` });

  it("inserts the largest variant within 1280 on the long edge", () => {
    expect(pickVariant([v(320, 180), v(1280, 720), v(2560, 1440)])?.url).toBe("1280x720.webp");
    expect(pickVariant([v(180, 320), v(720, 1280)])?.url).toBe("720x1280.webp");
    expect(pickVariant([v(200, 100)])?.url).toBe("200x100.webp");
  });

  it("falls back to the smallest one, or nothing", () => {
    expect(pickVariant([v(2000, 2000), v(3000, 3000)])?.url).toBe("2000x2000.webp");
    expect(pickVariant([])).toBeNull();
  });
});

describe("failureMessage", () => {
  it("translates the worker's reasons", () => {
    expect(failureMessage("corrupt")).toMatch(/损坏/);
    expect(failureMessage(null)).toMatch(/处理失败/);
    expect(failureMessage("weird")).toMatch(/处理失败/);
  });
});
