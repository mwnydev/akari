"use client";

// Tiptap 富文本编辑器（plan 第 4 节）：加粗、斜体、删除线、行内代码、代码块、引用、列表、标题、@提及。
// 只从按需加载的组件里引入（Composer、ListingForm），不进板块页与主题页的首屏 JS（plan 第 12 节）。
import { cn } from "@akari/ui";
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
import { filterMentionable, userName, type UserCardView } from "../../lib/display";
import { PROSE_CLASS } from "../../lib/prose";

interface MentionMenu {
  items: UserCardView[];
  selected: number;
  rect: DOMRect | null;
  pick: (u: UserCardView) => void;
}

export interface ForumEditorOptions {
  /** 初始内容（草稿或编辑器原稿）；为空时是空文档 */
  content: JSONContent | null;
  placeholder: string;
  mentionable: UserCardView[];
  onChange?: () => void;
}

/** 创建编辑器并返回 @提及菜单的状态（菜单由 <EditorField> 渲染）。 */
export function useForumEditor({
  content,
  placeholder,
  mentionable,
  onChange,
}: ForumEditorOptions) {
  const [menu, setMenu] = useState<MentionMenu | null>(null);
  // 扩展只在创建时配置一次，回调里通过 ref 读最新的值
  const latest = useRef({ menu, mentionable, onChange });
  useEffect(() => {
    latest.current = { menu, mentionable, onChange };
  });

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
      Mention.configure({
        suggestion: {
          char: "@",
          items: ({ query }) => filterMentionable(latest.current.mentionable, query),
          render: () => {
            const show = (p: SuggestionProps<UserCardView>) =>
              setMenu(
                p.items.length
                  ? {
                      items: p.items,
                      selected: 0,
                      rect: p.clientRect?.() ?? null,
                      pick: (u) => p.command({ id: String(u.id), label: userName(u) }),
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

  return { editor, menu };
}

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

function Toolbar({ editor }: { editor: Editor }) {
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
      {/* 图片上传接口（预签名直传 R2，plan 第 12 节）还没有，先占位 */}
      <ToolButton icon={ImagePlus} label="插入图片（即将开放）" disabled />
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
          @{userName(u)}
        </li>
      ))}
    </ul>
  );
}

/** 编辑框 + 工具栏 + 提及菜单。 */
export function EditorField({
  editor,
  menu,
  invalid,
}: {
  editor: Editor | null;
  menu: MentionMenu | null;
  invalid?: boolean;
}) {
  if (!editor) return null;
  return (
    <div
      className={cn(
        "rounded-md border bg-surface focus-within:outline-2 focus-within:outline-accent",
        invalid ? "border-danger" : "border-border",
      )}
    >
      <Toolbar editor={editor} />
      <EditorContent editor={editor} />
      {menu ? <MentionPopup menu={menu} /> : null}
    </div>
  );
}
