"use client";

// Tiptap 富文本编辑器（plan 第 4 节）：加粗、斜体、删除线、行内代码、代码块、引用、列表、标题、@提及、图片。
// 只从按需加载的组件里引入（Composer、ListingForm），不进板块页与主题页的首屏 JS（plan 第 12 节）。
import { errorMessage, type Schemas } from "@akari/api-client";
import { cn } from "@akari/ui";
import Image from "@tiptap/extension-image";
import Mention from "@tiptap/extension-mention";
import { Placeholder } from "@tiptap/extensions/placeholder";
import {
  EditorContent,
  useEditor,
  useEditorState,
  type Editor,
  type JSONContent,
} from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import type { SuggestionKeyDownProps, SuggestionProps } from "@tiptap/suggestion";
import {
  Bold,
  Code,
  Heading2,
  ImagePlus,
  Italic,
  List,
  ListOrdered,
  Quote,
  SquareCode,
  Strikethrough,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api, unwrap } from "../../lib/browser-api";
import type { UserCardView } from "../../lib/display";
import { mentionCandidates, mentionQuery, type MentionCandidate } from "../../lib/mentions";
import { PROSE_CLASS } from "../../lib/prose";
import {
  IMAGE_ACCEPT,
  imageProblem,
  pickVariant,
  uploadImage,
  type UploadProgress,
} from "../../lib/uploads";

interface MentionMenu {
  items: MentionCandidate[];
  selected: number;
  rect: DOMRect | null;
  pick: (u: MentionCandidate) => void;
}

type UploadStatus =
  | { kind: "busy"; name: string; progress: UploadProgress }
  | { kind: "error"; message: string }
  | null;

export interface ForumEditorOptions {
  /** 初始内容（草稿或编辑器原稿）；为空时是空文档 */
  content: JSONContent | null;
  placeholder: string;
  /** 本页出现过的人：输入 @ 后最先列出 */
  mentionable: UserCardView[];
  onChange?: () => void;
}

// 同一个前缀只搜一次（用户名搜索接口结果很稳定）；失败不影响继续打字
const searches = new Map<string, Promise<Schemas["MentionView"][]>>();
function searchUsers(q: string): Promise<Schemas["MentionView"][]> {
  let found = searches.get(q);
  if (!found) {
    found = unwrap(api.GET("/api/v1/users/search", { params: { query: { q } } })).catch(() => {
      searches.delete(q);
      return [];
    });
    searches.set(q, found);
  }
  return found;
}

/** 创建编辑器，并返回 @提及菜单与图片上传的状态（由 <EditorField> 渲染）。 */
export function useForumEditor({
  content,
  placeholder,
  mentionable,
  onChange,
}: ForumEditorOptions) {
  const [menu, setMenu] = useState<MentionMenu | null>(null);
  const [upload, setUpload] = useState<UploadStatus>(null);
  const uploadAbort = useRef<AbortController | null>(null);
  // 扩展只在创建时配置一次，回调里通过 ref 读最新的值
  const latest = useRef({ menu, mentionable, onChange });
  useEffect(() => {
    latest.current = { menu, mentionable, onChange };
  });
  useEffect(() => () => uploadAbort.current?.abort(), []);

  const editor = useEditor({
    // 组件只在浏览器里渲染（next/dynamic ssr: false），可以立即创建
    immediatelyRender: true,
    content,
    extensions: [
      StarterKit.configure({
        heading: { levels: [2, 3, 4] },
        link: { openOnClick: false, autolink: true },
      }),
      Placeholder.configure({ placeholder }),
      Image.configure({ allowBase64: false }),
      Mention.configure({
        suggestion: {
          char: "@",
          items: async ({ query }) => {
            const q = mentionQuery(query);
            const remote = q ? await searchUsers(q) : [];
            return mentionCandidates(latest.current.mentionable, remote, query);
          },
          render: () => {
            const show = (p: SuggestionProps<MentionCandidate>) =>
              setMenu(
                p.items.length
                  ? {
                      items: p.items,
                      selected: 0,
                      rect: p.clientRect?.() ?? null,
                      pick: (u) => p.command({ id: String(u.id), label: u.name }),
                    }
                  : null,
              );
            return {
              onStart: show,
              onUpdate: show,
              onExit: () => setMenu(null),
              onKeyDown: ({ event }: SuggestionKeyDownProps) => {
                const m = latest.current.menu;
                if (!m) return false;
                const n = m.items.length;
                if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                  const step = event.key === "ArrowDown" ? 1 : n - 1;
                  setMenu({ ...m, selected: (m.selected + step) % n });
                  return true;
                }
                if (event.key === "Enter" || event.key === "Tab") {
                  const u = m.items[m.selected];
                  if (u) m.pick(u);
                  return true;
                }
                if (event.key === "Escape") {
                  setMenu(null);
                  return true;
                }
                return false;
              },
            };
          },
        },
      }),
    ],
    editorProps: {
      attributes: {
        class: cn(PROSE_CLASS, "min-h-32 px-3 py-2 focus:outline-none"),
        role: "textbox",
        "aria-multiline": "true",
        "aria-label": "正文",
      },
    },
    onUpdate: () => latest.current.onChange?.(),
  });

  /** 选好图片：先在本地检查类型与大小，再直传、等服务器转成 WebP 后插入。 */
  async function insertImage(file: File) {
    const problem = imageProblem(file);
    if (problem) {
      setUpload({ kind: "error", message: problem });
      return;
    }
    uploadAbort.current?.abort();
    const abort = (uploadAbort.current = new AbortController());
    const name = file.name;
    setUpload({ kind: "busy", name, progress: { phase: "uploading", fraction: 0 } });
    try {
      const done = await uploadImage(
        file,
        (progress) => setUpload({ kind: "busy", name, progress }),
        abort.signal,
      );
      const v = pickVariant(done.variants);
      if (!v) throw new Error("no variants");
      editor
        ?.chain()
        .focus()
        .setImage({ src: v.url, alt: "", width: v.width, height: v.height })
        .run();
      setUpload(null);
    } catch (e) {
      if (abort.signal.aborted) return;
      setUpload({ kind: "error", message: errorMessage(e) });
    }
  }

  return { editor, menu, upload, insertImage };
}

export type ForumEditor = ReturnType<typeof useForumEditor>;

function ToolButton({
  icon: Icon,
  label,
  active,
  disabled,
  onClick,
}: {
  icon: LucideIcon;
  label: string;
  active?: boolean;
  disabled?: boolean;
  onClick?: () => void;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      aria-pressed={active}
      disabled={disabled}
      onClick={onClick}
      className={cn(
        "inline-flex size-8 items-center justify-center rounded-md text-fg-muted hover:bg-surface-2 hover:text-fg",
        "disabled:pointer-events-none disabled:opacity-40",
        active && "bg-accent-soft text-accent",
      )}
    >
      <Icon size={16} aria-hidden />
    </button>
  );
}

function Toolbar({
  editor,
  uploading,
  onPickImage,
}: {
  editor: Editor;
  uploading: boolean;
  onPickImage: () => void;
}) {
  const s = useEditorState({
    editor,
    selector: ({ editor: e }) => ({
      bold: e.isActive("bold"),
      italic: e.isActive("italic"),
      strike: e.isActive("strike"),
      code: e.isActive("code"),
      heading: e.isActive("heading", { level: 2 }),
      quote: e.isActive("blockquote"),
      bullet: e.isActive("bulletList"),
      ordered: e.isActive("orderedList"),
      codeBlock: e.isActive("codeBlock"),
    }),
  });
  const chain = () => editor.chain().focus();
  return (
    <div
      role="toolbar"
      aria-label="格式"
      className="flex flex-wrap items-center gap-0.5 border-b border-border px-1 py-1"
    >
      <ToolButton
        icon={Bold}
        label="加粗"
        active={s.bold}
        onClick={() => chain().toggleBold().run()}
      />
      <ToolButton
        icon={Italic}
        label="斜体"
        active={s.italic}
        onClick={() => chain().toggleItalic().run()}
      />
      <ToolButton
        icon={Strikethrough}
        label="删除线"
        active={s.strike}
        onClick={() => chain().toggleStrike().run()}
      />
      <ToolButton
        icon={Code}
        label="行内代码"
        active={s.code}
        onClick={() => chain().toggleCode().run()}
      />
      <ToolButton
        icon={Heading2}
        label="小标题"
        active={s.heading}
        onClick={() => chain().toggleHeading({ level: 2 }).run()}
      />
      <ToolButton
        icon={Quote}
        label="引用"
        active={s.quote}
        onClick={() => chain().toggleBlockquote().run()}
      />
      <ToolButton
        icon={List}
        label="无序列表"
        active={s.bullet}
        onClick={() => chain().toggleBulletList().run()}
      />
      <ToolButton
        icon={ListOrdered}
        label="有序列表"
        active={s.ordered}
        onClick={() => chain().toggleOrderedList().run()}
      />
      <ToolButton
        icon={SquareCode}
        label="代码块"
        active={s.codeBlock}
        onClick={() => chain().toggleCodeBlock().run()}
      />
      <ToolButton icon={ImagePlus} label="插入图片" disabled={uploading} onClick={onPickImage} />
    </div>
  );
}

function MentionPopup({ menu }: { menu: MentionMenu }) {
  const rect = menu.rect;
  return (
    <ul
      role="listbox"
      aria-label="提及用户"
      className="glass-strong fixed z-50 max-h-64 min-w-40 overflow-auto rounded-lg p-1 shadow-sheet"
      style={rect ? { top: rect.bottom + 4, left: rect.left } : { display: "none" }}
    >
      {menu.items.map((u, i) => (
        <li
          key={u.id}
          role="option"
          aria-selected={i === menu.selected}
          // mousedown 而不是 click：避免编辑器先失焦把提及菜单关掉
          onMouseDown={(e) => {
            e.preventDefault();
            menu.pick(u);
          }}
          className={cn(
            "cursor-pointer rounded-md px-3 py-1.5 text-sm",
            i === menu.selected ? "bg-accent-soft text-accent" : "hover:bg-surface-2",
          )}
        >
          @{u.name}
        </li>
      ))}
    </ul>
  );
}

function UploadLine({ status }: { status: NonNullable<UploadStatus> }) {
  if (status.kind === "error") {
    return (
      <p role="alert" className="border-t border-border px-3 py-1.5 text-sm text-danger">
        {status.message}
      </p>
    );
  }
  const { phase, fraction } = status.progress;
  const pct = Math.round(fraction * 100);
  return (
    <div className="border-t border-border px-3 py-1.5 text-sm text-fg-muted" aria-live="polite">
      <span className="flex justify-between gap-2">
        <span className="truncate">{status.name}</span>
        <span className="amount shrink-0">
          {phase === "uploading" ? `上传中 ${pct}%` : "正在转换为 WebP…"}
        </span>
      </span>
      <progress
        className="mt-1 h-1 w-full accent-accent"
        aria-label="图片上传进度"
        max={100}
        value={phase === "uploading" ? pct : undefined}
      />
    </div>
  );
}

/** 编辑框 + 工具栏 + 提及菜单 + 图片上传进度。 */
export function EditorField({
  editor,
  menu,
  upload,
  insertImage,
  invalid,
}: ForumEditor & { invalid?: boolean }) {
  const fileRef = useRef<HTMLInputElement>(null);
  if (!editor) return null;
  return (
    <div
      className={cn(
        "rounded-md border bg-surface focus-within:outline-2 focus-within:outline-accent",
        invalid ? "border-danger" : "border-border",
      )}
    >
      <Toolbar
        editor={editor}
        uploading={upload?.kind === "busy"}
        onPickImage={() => fileRef.current?.click()}
      />
      <input
        ref={fileRef}
        type="file"
        accept={IMAGE_ACCEPT}
        className="hidden"
        data-testid="image-input"
        onChange={(e) => {
          const file = e.target.files?.[0];
          e.target.value = "";
          if (file) void insertImage(file);
        }}
      />
      <EditorContent editor={editor} />
      {upload ? <UploadLine status={upload} /> : null}
      {menu ? <MentionPopup menu={menu} /> : null}
    </div>
  );
}
