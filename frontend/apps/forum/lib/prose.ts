/** 帖子正文排版：服务端渲染的楼层、浏览器里插入的编辑历史与编辑器共用。 */
export const PROSE_CLASS =
  "space-y-3 break-words [&_a]:text-accent [&_a]:underline [&_blockquote]:border-l-4 [&_blockquote]:border-border " +
  "[&_blockquote]:pl-3 [&_blockquote]:text-fg-muted [&_code]:rounded [&_code]:bg-surface-2 [&_code]:px-1 " +
  "[&_h2]:text-xl [&_h2]:font-semibold [&_h3]:text-lg [&_h3]:font-semibold [&_h4]:font-semibold " +
  "[&_img]:max-w-full [&_img]:rounded-md [&_ol]:ml-5 [&_ol]:list-decimal [&_pre]:overflow-x-auto " +
  "[&_pre]:rounded-md [&_pre]:bg-surface-2 [&_pre]:p-3 [&_pre_code]:bg-transparent [&_pre_code]:p-0 " +
  "[&_span[data-type=mention]]:font-medium [&_span[data-type=mention]]:text-accent [&_strong]:font-semibold " +
  "[&_ul]:ml-5 [&_ul]:list-disc";
