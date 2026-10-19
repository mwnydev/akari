// 从 Mailpit（本地截获邮件）取最新的 6 位验证码。
const MAILPIT = process.env.MAILPIT_URL ?? "http://127.0.0.1:8025";

interface SearchResult {
  messages: { ID: string }[];
}

export async function latestCode(to: string, timeoutMs = 15_000): Promise<string> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const res = await fetch(`${MAILPIT}/api/v1/search?query=${encodeURIComponent(`to:${to}`)}`);
    const found = (await res.json()) as SearchResult;
    const id = found.messages[0]?.ID;
    if (id) {
      const msg = (await (await fetch(`${MAILPIT}/api/v1/message/${id}`)).json()) as {
        Text: string;
      };
      const code = /\b(\d{6})\b/.exec(msg.Text)?.[1];
      if (code) return code;
    }
    await new Promise((r) => setTimeout(r, 300));
  }
  throw new Error(`no login code mailed to ${to}`);
}
