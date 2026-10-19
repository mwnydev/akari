import { Tabs, TabsContent, TabsList, TabsTrigger } from "@akari/ui/tabs";
import type { Metadata } from "next";
import { hotTopics, topics } from "./_mock/data";
import { BoardDirectory } from "./_components/BoardDirectory";
import { ForumShell } from "./_components/ForumShell";
import { ListToolbar } from "./_components/ListToolbar";
import { TopicList } from "./_components/TopicList";

export const metadata: Metadata = { title: "首页" };

export default function DesignHomePage() {
  return (
    <ForumShell page="/design/v2" active="home">
      <h1 className="sr-only">Akari 论坛</h1>
      <Tabs defaultValue="latest">
        <ListToolbar newLabel="发新帖">
          <TabsList variant="underline" aria-label="主题排序">
            <TabsTrigger variant="underline" value="latest">
              最新
            </TabsTrigger>
            <TabsTrigger variant="underline" value="hot">
              热门
            </TabsTrigger>
            <TabsTrigger variant="underline" value="boards">
              板块
            </TabsTrigger>
          </TabsList>
        </ListToolbar>
        <TabsContent value="latest">
          <TopicList topics={topics} label="最新主题" />
        </TabsContent>
        <TabsContent value="hot">
          <TopicList topics={hotTopics} label="热门主题" />
        </TabsContent>
        <TabsContent value="boards">
          <BoardDirectory />
        </TabsContent>
      </Tabs>
    </ForumShell>
  );
}
