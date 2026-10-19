// 服务端配置（只在服务器读取，不进浏览器包）。

/** forum-api 源站地址：服务端渲染直连；浏览器走同源 /api/v1（开发环境由 Next rewrite 转发，生产由 Caddy 转发）。 */
export const API_ORIGIN = process.env.API_ORIGIN ?? "http://127.0.0.1:8080";

/** 会话 Cookie 名，与后端 `forum_api::SESSION_COOKIE` 一致。 */
export const SESSION_COOKIE = "sid";
