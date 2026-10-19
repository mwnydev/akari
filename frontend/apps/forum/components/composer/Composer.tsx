"use client";

import { errorMessage } from "@akari/api-client";
import { Button, ErrorState, Field, Input, LoadingState } from "@akari/ui";
import { useMutation, useQuery } from "@tanstack/react-query";
import type { JSONContent } from "@tiptap/react";
import { useEffect, useRef, useState } from "react";
import { api, unwrap } from "../../lib/browser-api";
import { clearDraft, loadDraft, saveDraft, type Draft } from "../../lib/drafts";
import type { UserCardView } from "../../lib/display";
import { topicHref } from "../../lib/links";
import { QueryProvider } from "../QueryProvider";
import { EditorField, useForumEditor } from "./Editor";
import { draftKey, type ComposeTarget } from "./target";

const AUTOSAVE_DELAY_MS = 600;

export interface ComposerProps {
  target: ComposeTarget;
  /** @提及的候选人 */
  mentionable: UserCardView[];
  /** 提交成功：发主题、回帖给出要跳转的地址，编辑为 null（原地刷新） */
  onDone: (href: string | null) => void;
  onCancel: () => void;
}

const SUBMIT_LABEL: Record<ComposeTarget["kind"], [idle: string, pending: string]> = {
  topic: ["发布主题", "发布中…"],
  reply: ["发表回复", "发送中…"],
  edit: ["保存修改", "保存中…"],
};

const PLACEHOLDER: Record<ComposeTarget["kind"], string> = {
  topic: "正文：支持 @提及、代码块；草稿会自动保存在本机",
  reply: "写下你的回复，输入 @ 提及楼里的人",
  edit: "修改内容；编辑前的版本会保留在编辑历史里",
};

function savedAtLabel(t: number): string {
  return new Date(t).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" });
}

async function submit(
  target: ComposeTarget,
  body: { title: string; html: string; source: JSONContent },
): Promise<string | null> {
  const { html, source } = body;
  switch (target.kind) {
    case "topic": {
      const created = await unwrap(
        api.POST("/api/v1/forums/{slug}/topics", {
          params: { path: { slug: target.forumSlug } },
          body: { title: body.title, html, source },
        }),
      );
      return topicHref(created.topic_id);
    }
    case "reply": {
      const created = await unwrap(
        api.POST("/api/v1/topics/{id}/posts", {
          params: { path: { id: target.topicId } },
          body: { html, source },
        }),
      );
      return topicHref(target.topicId, created.page, created.floor);
    }
    case "edit":
      await unwrap(
        api.PATCH("/api/v1/posts/{id}", {
          params: { path: { id: target.postId } },
          body: { html, source, title: target.floor === 1 ? body.title : null },
        }),
      );
      return null;
  }
}

function ComposerForm({
  target,
  mentionable,
  onDone,
  onCancel,
  initial,
}: ComposerProps & {
  initial: { title: string; doc: JSONContent | null; savedAt: number | null };
}) {
  const key = draftKey(target);
  const withTitle = target.kind === "topic" || (target.kind === "edit" && target.floor === 1);
  const [title, setTitle] = useState(initial.title);
  const [savedAt, setSavedAt] = useState(initial.savedAt);
  const [localError, setLocalError] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const titleRef = useRef(title);

  const { editor, menu } = useForumEditor({
    content: initial.doc,
    placeholder: PLACEHOLDER[target.kind],
    mentionable,
    onChange: () => scheduleSave(),
  });

  function scheduleSave() {
    clearTimeout(timer.current);
    timer.current = setTimeout(() => {
      if (!editor) return;
      const draft: Draft = { doc: editor.getJSON(), savedAt: Date.now() };
      if (withTitle) draft.title = titleRef.current;
      if (saveDraft(key, draft)) setSavedAt(draft.savedAt);
    }, AUTOSAVE_DELAY_MS);
  }
  useEffect(() => () => clearTimeout(timer.current), []);

  const mutation = useMutation({
    mutationFn: () => {
      if (!editor) throw new Error("editor not ready");
      return submit(target, {
        title: title.trim(),
        html: editor.getHTML(),
        source: editor.getJSON(),
      });
    },
    onSuccess: (href) => {
      clearTimeout(timer.current);
      clearDraft(key);
      onDone(href);
    },
  });

  function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (withTitle && !title.trim()) return setLocalError("请填写标题");
    if (!editor || editor.isEmpty) return setLocalError("内容不能为空");
    setLocalError(null);
    mutation.mutate();
  }

  function discard() {
    clearTimeout(timer.current);
    clearDraft(key);
    setSavedAt(null);
    onCancel();
  }

  const [idle, pending] = SUBMIT_LABEL[target.kind];
  const error = localError ?? (mutation.isError ? errorMessage(mutation.error) : null);
  return (
    <form className="space-y-3" onSubmit={onSubmit} noValidate aria-label={idle}>
      {withTitle ? (
        <Field label="标题">
          <Input
            value={title}
            maxLength={120}
            placeholder="1–120 个字"
            onChange={(e) => {
              setTitle(e.target.value);
              titleRef.current = e.target.value;
              scheduleSave();
            }}
          />
        </Field>
      ) : null}
      <EditorField editor={editor} menu={menu} invalid={localError === "内容不能为空"} />
      <p className="text-xs text-fg-muted">图片上传即将开放；现在可以粘贴文字、链接和代码。</p>
      {error ? (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      ) : null}
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="text-xs text-fg-muted" aria-live="polite">
          {savedAt ? `草稿已自动保存 ${savedAtLabel(savedAt)}` : ""}
        </span>
        <div className="flex gap-2">
          <Button variant="ghost" onClick={savedAt ? discard : onCancel}>
            {savedAt ? "放弃草稿" : "取消"}
          </Button>
          <Button type="submit" disabled={mutation.isPending || !editor}>
            {mutation.isPending ? pending : idle}
          </Button>
        </div>
      </div>
    </form>
  );
}

/** 编辑已有楼层：本机有未提交的修改就接着改，否则从后端取编辑器原稿。 */
function EditLoader(props: ComposerProps & { target: Extract<ComposeTarget, { kind: "edit" }> }) {
  const { target } = props;
  const [draft] = useState(() => loadDraft(draftKey(target)));
  const source = useQuery({
    queryKey: ["post-source", target.postId],
    queryFn: () =>
      unwrap(api.GET("/api/v1/posts/{id}/source", { params: { path: { id: target.postId } } })),
    enabled: draft === null,
    staleTime: 0,
    gcTime: 0,
  });
  if (draft) {
    return (
      <ComposerForm
        {...props}
        initial={{
          title: draft.title ?? target.title ?? "",
          doc: draft.doc,
          savedAt: draft.savedAt,
        }}
      />
    );
  }
  if (source.isPending) return <LoadingState rows={2} label="正在加载原稿" />;
  if (source.isError) {
    return (
      <ErrorState
        title="原稿加载失败"
        message={errorMessage(source.error)}
        action={
          <Button variant="secondary" size="sm" onClick={props.onCancel}>
            关闭
          </Button>
        }
      />
    );
  }
  const doc = source.data.source as JSONContent;
  return <ComposerForm {...props} initial={{ title: target.title ?? "", doc, savedAt: null }} />;
}

function NewContent(props: ComposerProps) {
  const [draft] = useState(() => loadDraft(draftKey(props.target)));
  return (
    <ComposerForm
      {...props}
      initial={{
        title: draft?.title ?? "",
        doc: draft?.doc ?? null,
        savedAt: draft?.savedAt ?? null,
      }}
    />
  );
}

/** 发主题 / 回帖 / 编辑的编辑器（按需加载，见 ComposerLauncher）。 */
export default function Composer(props: ComposerProps) {
  const { target } = props;
  return (
    <QueryProvider>
      {target.kind === "edit" ? (
        <EditLoader {...props} target={target} />
      ) : (
        <NewContent {...props} />
      )}
    </QueryProvider>
  );
}
