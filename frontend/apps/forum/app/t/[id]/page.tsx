import { Badge, Button, Card, Textarea, Toolbar } from "@akari/ui";
import { Bookmark, CornerDownRight, ListOrdered, Share } from "lucide-react";
import { SiteNav } from "../../../components/SiteNav";
import { Floor } from "../../../components/Floor";
import { floors, topics } from "../../../lib/mock";

export default async function TopicPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const topic = topics.find((t) => String(t.id) === id) ?? topics[1];
  return (
    <>
      <SiteNav />
      <main className="mx-auto max-w-3xl space-y-3 px-4 py-5">
        <nav aria-label="位置" className="text-sm text-fg-muted">
          <a href="/" className="hover:text-fg">
            首页
          </a>{" "}
          /{" "}
          <a href={`/f/${topic?.board ?? "chat"}`} className="hover:text-fg">
            综合讨论
          </a>
        </nav>
        <h1 className="text-2xl leading-snug font-semibold">
          {topic?.digest ? (
            <Badge tone="success" className="mr-2 align-[4px]">
              精华
            </Badge>
          ) : null}
          {topic?.title}
        </h1>
        <p className="amount text-sm text-fg-muted">
          {topic?.views.toLocaleString("zh-CN")} 浏览 · {topic?.replies} 回复 · 第 1 / 7 页
        </p>
        {floors.map((f) => (
          <Floor key={f.floor} f={f} />
        ))}
        <nav aria-label="分页" className="flex flex-wrap items-center justify-center gap-1 py-2">
          {[1, 2, 3, "…", 7].map((p, i) => (
            <Button
              key={i}
              size="sm"
              variant={p === 1 ? "primary" : "secondary"}
              className="min-w-9 px-2"
              disabled={p === "…"}
            >
              {p}
            </Button>
          ))}
        </nav>
        <Card>
          <label htmlFor="reply" className="mb-2 block font-medium">
            回复
          </label>
          <Textarea
            id="reply"
            placeholder="支持 @提及、引用楼层、图片（Tiptap 编辑器接入后替换）"
          />
          <div className="mt-3 flex items-center justify-between">
            <span className="text-xs text-fg-muted">草稿已自动保存</span>
            <Button>发表回复</Button>
          </div>
        </Card>
      </main>
      <div className="fixed inset-x-0 bottom-4 z-20 flex justify-center px-4">
        <Toolbar aria-label="帖子操作">
          <Button variant="ghost" size="sm">
            <CornerDownRight size={16} aria-hidden />
            回复
          </Button>
          <Button variant="ghost" size="sm">
            <Bookmark size={16} aria-hidden />
            收藏
          </Button>
          <Button variant="ghost" size="sm">
            <Share size={16} aria-hidden />
            分享
          </Button>
          <Button variant="ghost" size="sm">
            <ListOrdered size={16} aria-hidden />
            跳楼
          </Button>
        </Toolbar>
      </div>
    </>
  );
}
