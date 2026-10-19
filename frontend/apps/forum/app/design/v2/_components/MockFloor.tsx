import type { ReactNode } from "react";
import { Floor } from "../../../../components/forum/Floor";
import { FloorContent } from "../../../../components/forum/FloorContent";
import type { FloorData } from "../_mock/data";
import { mockBody, mockCanModerate, mockPerson, mockSocial, mockViewer } from "../_mock/views";

/** 一层模拟楼层：正式的 Floor + FloorContent。`extra` 放嵌入正文的内容（报价卡片等）。 */
export function MockFloor({
  data: f,
  page,
  opAuthorId,
  extra,
}: {
  data: FloorData;
  /** 设计稿页面路径：引用块的链接要写成站内路径 */
  page: string;
  opAuthorId: number;
  extra?: ReactNode;
}) {
  const author = mockPerson(f.authorId);
  return (
    <Floor
      floor={f.floor}
      author={author}
      isOp={f.authorId === opAuthorId}
      timeIso={f.timeIso}
      time={f.time}
      edited={f.edited ? "已编辑" : undefined}
    >
      <FloorContent
        topicId={0}
        postId={f.floor}
        floor={f.floor}
        author={author.name}
        quoteHref={`${page}#floor-${f.floor}`}
        social={mockSocial(f)}
        viewer={mockViewer}
        canModerate={mockCanModerate}
        canReply
        extra={extra}
      >
        {mockBody(f)}
      </FloorContent>
    </Floor>
  );
}
