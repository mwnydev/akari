// 「引用」的摘录（纯函数，单元测试直接测）。

/** 引用摘录最长字数（按字符，不截断汉字或表情的代理对）。 */
export const QUOTE_CHARS = 140;

/**
 * 一层楼自己写的字：去掉正文里的引用块（它引用的别的楼层）再取纯文本，压成一行，最多 QUOTE_CHARS 字。
 * 引用一层带引用的楼不会一层套一层（Discourse 同样不嵌套引用）。会改动传入的节点，调用方传副本。
 */
export function ownText(node: Node): string {
  if ("querySelectorAll" in node) {
    (node as ParentNode).querySelectorAll("blockquote").forEach((q) => q.remove());
  }
  const text = (node.textContent ?? "").replace(/\s+/g, " ").trim();
  const chars = [...text];
  return chars.length > QUOTE_CHARS ? `${chars.slice(0, QUOTE_CHARS).join("")}…` : text;
}
