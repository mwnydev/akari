"use client";

import { errorMessage } from "@akari/api-client";
import { Button, EmptyState, ErrorState, LoadingState } from "@akari/ui";
import { Dialog, DialogContent } from "@akari/ui/dialog";
import { useInfiniteQuery } from "@tanstack/react-query";
import { api, unwrap } from "../../lib/browser-api";
import { formatTime } from "../../lib/display";
import { PROSE_CLASS } from "../../lib/prose";
import { sanitizeHtml } from "../../lib/sanitize.client";
import { QueryProvider } from "../QueryProvider";

interface Props {
  postId: number;
  floor: number;
  onClose: () => void;
}

function History({ postId }: { postId: number }) {
  const q = useInfiniteQuery({
    queryKey: ["post-history", postId],
    initialPageParam: undefined as number | undefined,
    queryFn: ({ pageParam }) =>
      unwrap(
        api.GET("/api/v1/posts/{id}/history", {
          params: { path: { id: postId }, query: pageParam ? { cursor: pageParam } : {} },
        }),
      ),
    getNextPageParam: (last) => (last.next_cursor ? Number(last.next_cursor) : undefined),
  });
  if (q.isPending) return <LoadingState rows={2} label="正在加载编辑历史" />;
  if (q.isError) return <ErrorState message={errorMessage(q.error)} />;
  const edits = q.data.pages.flatMap((p) => p.edits);
  if (!edits.length) return <EmptyState title="没有编辑记录" />;
  return (
    <div className="max-h-[60vh] space-y-4 overflow-y-auto">
      {edits.map((e) => (
        <article key={e.id} className="rounded-lg bg-surface-2 p-3">
          <p className="mb-2 text-xs text-fg-muted">
            <time dateTime={e.edited_at}>{formatTime(e.edited_at)}</time> 之前的版本
            {e.title ? ` · 原标题「${e.title}」` : ""}
          </p>
          {/* 接口返回的 HTML 已由后端过滤，插入前再过一次 DOMPurify */}
          <div className={PROSE_CLASS} dangerouslySetInnerHTML={{ __html: sanitizeHtml(e.html) }} />
        </article>
      ))}
      {q.hasNextPage ? (
        <div className="flex justify-center">
          <Button
            variant="secondary"
            size="sm"
            disabled={q.isFetchingNextPage}
            onClick={() => void q.fetchNextPage()}
          >
            {q.isFetchingNextPage ? "加载中…" : "更早的版本"}
          </Button>
        </div>
      ) : null}
    </div>
  );
}

export default function HistoryDialog({ postId, floor, onClose }: Props) {
  return (
    <QueryProvider>
      <Dialog open onOpenChange={(o) => (o ? undefined : onClose())}>
        <DialogContent title={`#${floor} 编辑历史`} description="新的在前" className="max-w-2xl">
          <History postId={postId} />
        </DialogContent>
      </Dialog>
    </QueryProvider>
  );
}
