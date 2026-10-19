import { describe, expect, it, vi } from "vitest";

vi.mock("server-only", () => ({}));
const { sanitizeHtml } = await import("./sanitize.server");

describe("sanitizeHtml", () => {
  it("keeps what the backend allows", () => {
    const html =
      "<p>hi <strong>there</strong></p>" +
      '<pre><code class="language-rust">fn main() {}</code></pre>' +
      '<p><a href="https://example.com" rel="nofollow ugc noopener noreferrer">link</a></p>' +
      '<p><img src="https://img.example/a.webp" alt="" width="1280" height="720"></p>';
    expect(sanitizeHtml(html)).toBe(html);
  });

  it("turns mentions into profile links by user ID", () => {
    expect(
      sanitizeHtml(
        '<p>hi <span data-type="mention" data-id="7" data-label="小林">@小林</span></p>',
      ),
    ).toBe('<p>hi <a href="/u/7" data-type="mention">@小林</a></p>');
  });

  it("does not let a mention label inject markup or a link target", () => {
    const out = sanitizeHtml(
      '<span data-type="mention" data-id="7" data-label="x"><img src=x onerror=alert(1)>@&lt;b&gt;</span>' +
        '<span data-type="mention" data-id="javascript:alert(1)">@y</span>',
    );
    expect(out).toBe(
      '<a href="/u/7" data-type="mention">@&lt;b&gt;</a><span data-type="mention">@y</span>',
    );
  });

  it("strips scripts, handlers, styles and dangerous URLs", () => {
    const out = sanitizeHtml(
      '<p onclick="x()" style="color:red">a<script>alert(1)</script></p>' +
        '<img src="x" onerror="alert(1)"><a href="javascript:alert(1)">b</a><iframe src="https://e.com"></iframe>',
    );
    expect(out).not.toMatch(/script|onclick|onerror|style|javascript|iframe/);
    expect(out).toContain("<p>a</p>");
  });

  it("drops data attributes outside the mention whitelist", () => {
    expect(sanitizeHtml('<span data-type="mention" data-evil="1">@x</span>')).toBe(
      '<span data-type="mention">@x</span>',
    );
  });
});
