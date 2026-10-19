"use client";

import { errorMessage } from "@akari/api-client";
import { Button, Field, Textarea } from "@akari/ui";
import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation } from "@tanstack/react-query";
import { ImagePlus, X } from "lucide-react";
import { useRef, useState } from "react";
import { useForm } from "react-hook-form";
import * as z from "zod/mini";
import { httpUrl } from "../../lib/arbitration";
import { api, unwrap } from "../../lib/browser-api";
import { IMAGE_ACCEPT, uploadImage, type UploadView } from "../../lib/uploads";

/** 与后端 `MAX_STATEMENT_UPLOADS`、`MAX_TEXT_CHARS` 一致。 */
const MAX_IMAGES = 9;

const Schema = z.object({
  body: z.string().check(
    z.refine((v) => v.trim().length > 0, { error: "请写下陈述" }),
    z.maxLength(2000, { error: "最多 2000 字" }),
  ),
});

/** 当事人在举证期内追加陈述与图片证据（只追加，双方与仲裁员都能看到）。 */
export default function StatementForm({ caseId, onDone }: { caseId: number; onDone: () => void }) {
  const form = useForm<z.infer<typeof Schema>>({
    resolver: standardSchemaResolver(Schema),
    defaultValues: { body: "" },
  });
  const [images, setImages] = useState<UploadView[]>([]);
  const fileInput = useRef<HTMLInputElement>(null);
  const upload = useMutation({
    mutationFn: (file: File) => uploadImage(file, () => undefined, new AbortController().signal),
    onSuccess: (u) => setImages((xs) => [...xs, u]),
  });
  const submit = useMutation({
    mutationFn: (v: z.infer<typeof Schema>) =>
      unwrap(
        api.POST("/api/v1/arbitration/cases/{id}/statements", {
          params: { path: { id: caseId } },
          body: { body: v.body.trim(), upload_ids: images.map((i) => i.id) },
        }),
      ),
    onSuccess: () => {
      form.reset();
      setImages([]);
      onDone();
    },
  });
  return (
    <form
      className="space-y-2"
      noValidate
      aria-label="提交陈述与证据"
      onSubmit={form.handleSubmit((v) => submit.mutate(v))}
    >
      <Field label="陈述（只追加，提交后不能修改）" error={form.formState.errors.body?.message}>
        <Textarea maxLength={2000} {...form.register("body")} />
      </Field>
      {images.length ? (
        <ul className="flex flex-wrap gap-2">
          {images.map((img) => (
            <li key={img.id} className="relative">
              <img
                src={httpUrl(img.variants[0]?.url) ?? undefined}
                alt="证据图片"
                className="size-16 rounded-md object-cover"
              />
              <button
                type="button"
                className="absolute -top-1 -right-1 rounded-full bg-surface p-0.5 shadow"
                onClick={() => setImages((xs) => xs.filter((x) => x.id !== img.id))}
              >
                <X size={12} aria-hidden />
                <span className="sr-only">移除</span>
              </button>
            </li>
          ))}
        </ul>
      ) : null}
      <input
        ref={fileInput}
        type="file"
        accept={IMAGE_ACCEPT}
        hidden
        onChange={(e) => {
          const f = e.target.files?.[0];
          if (f) upload.mutate(f);
          e.target.value = "";
        }}
      />
      {upload.isError || submit.isError ? (
        <p role="alert" className="text-sm text-danger">
          {errorMessage(upload.error ?? submit.error)}
        </p>
      ) : null}
      <div className="flex flex-wrap gap-2">
        <Button type="submit" disabled={submit.isPending || upload.isPending}>
          {submit.isPending ? "提交中…" : "提交陈述"}
        </Button>
        <Button
          variant="secondary"
          disabled={upload.isPending || images.length >= MAX_IMAGES}
          onClick={() => fileInput.current?.click()}
        >
          <ImagePlus size={16} aria-hidden />
          {upload.isPending ? "上传中…" : `添加图片（${images.length}/${MAX_IMAGES}）`}
        </Button>
      </div>
    </form>
  );
}
