/** 帖子正文：HTML 在发帖时已由后端 ammonia 过滤（plan 第 11 节），这里只是模拟数据。 */
export function Prose({ html }: { html: string }) {
  return (
    <div
      className="space-y-3 break-words [&_li]:ml-5 [&_li]:list-disc [&_strong]:font-semibold"
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}
