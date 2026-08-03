# 测试服务器 Docker 部署方案

目标机：`172.104.94.251`（Linode 东京，Ubuntu 24.04.3 LTS）
适用版本：本仓库 `imleoo/kiro.rs-admin` v0.7.4
文档日期：2026-08-03

---

## 一、服务器现状（实测）

| 项目 | 实测值 | 结论 |
|---|---|---|
| OS / 架构 | Ubuntu 24.04.3 LTS / x86_64 | 匹配 `linux/amd64` 镜像 |
| CPU / 内存 | 8 核 / 15 GiB（可用 14 GiB） | 足够在本机直接编译 Rust |
| 磁盘 | 315 G，已用 24 G（8%） | 充裕 |
| Docker | Server 29.1.3 | ✅ |
| Docker Compose | v5.0.1（`docker compose` 子命令） | ✅ |
| 已运行容器 | `sub2api`(8080)、`sub2api-redis`、`sub2api-postgres`、`hysteria` | 端口不冲突 |
| 宿主监听 | 22、80、443、888、8080、24744(宝塔)、5432(仅本地) | **8990 空闲** |
| 反向代理 | 宝塔 nginx，vhost 目录 `/www/server/panel/vhost/nginx/`，已有 `ai.leoobai.cn` 等站点 | 可复用做 TLS 入口 |
| 防火墙 | UFW active，`INPUT` 默认 DROP；放行 20/21/22/80/443/888/8443/16861/24744/39000-40000 | **8990 未放行**（本方案也不需要放行） |
| 出网 | `api.github.com` 200；`ghcr.io` 401（正常未认证响应）；`codewhisperer.us-east-1.amazonaws.com`、`q.us-east-1.amazonaws.com` 404（可达） | 直连上游可用，**无需配代理** |
| 基础镜像可拉取 | `rust:1.92-alpine`、`oven/bun:1-alpine`、`alpine:3.21` 均 OK | 本机构建可行 |

---

## 二、三条必须先明确的风险（部署前决策）

### 🔴 风险 1：仓库自带的 `docker-compose.yml` 默认拉的是**上游镜像**

```yaml
image: ${KIRO_RS_IMAGE:-zyphrzero/kiro-rs:latest}   # ← 上游 ZyphrZero 的镜像
```

`zyphrzero/kiro-rs` 是上游 `ZyphrZero/kiro.rs` 的产物，**不包含本仓库的任何自定义功能**
（企业 SSO、代理池负载均衡、多端点降级链 + 账号级 429 冷却、自定义模型映射、trace 请求日志等，
详见 `docs/CUSTOM_FEATURES.md`）。直接 `docker compose up -d` 会跑起一个"看起来像但功能不对"的服务。

**决策：不使用该默认镜像。** 采用下面方案 A（服务器本地源码构建）。

### 🔴 风险 2：Admin 面板的"在线更新"会把二进制换成**上游版本**

`src/admin/binary_update.rs:26` 与 `src/admin/service.rs:580` 中硬编码：

```rust
const GITHUB_REPO: &str = "ZyphrZero/kiro.rs";
```

在线更新从上游 Release 下载 musl 二进制替换 `/app/kiro-rs`，进程退出后由 `restart: unless-stopped`
拉起——**结果是自定义功能全部消失**，且配置结构可能不兼容。

**决策（本次部署采用）：**
- `config.json` 保持 `"updateAutoApply": false`（默认值，不要改成 true）；
- 运营上约定：**永不点击 Admin 面板的"更新/升级"按钮**；
- 更新一律走本方案第五节的「重新构建镜像」流程。

> 长期建议（不属于本次部署范围）：把这两处常量改成 `imleoo/kiro.rs-admin`，并在本仓库打 `vX.Y.Z`
> tag 触发 `release.yaml` 产出 musl 二进制，在线更新才安全可用。

### 🟡 风险 3：Docker 端口映射会**绕过 UFW**

宿主 `iptables -P INPUT DROP` + UFW 只作用于 `INPUT` 链，而 Docker 的发布端口走
`nat PREROUTING → FORWARD`，`DOCKER-USER` 链当前为空（默认放行）。
若写成 `ports: - "8990:8990"`，8990 会**直接暴露到公网**，无视 UFW 未放行的事实。

**决策：绑定到回环** `127.0.0.1:8990:8990`，公网访问统一走宿主 nginx（已有 TLS 与证书管理）。

---

## 三、方案选型

| 方案 | 做法 | 取舍 |
|---|---|---|
| **A. 服务器本地源码构建（推荐）** | git clone 本仓库 → `docker compose build` → 起容器 | 不依赖任何 registry，版本与 master 完全一致；首次全量编译较慢（数百 crate，8 核上十几分钟量级），后续改代码仍会全量重编译（Dockerfile 无依赖缓存层） |
| B. GitHub Actions → GHCR → 服务器 pull | 推 master 触发 `.github/workflows/docker-build.yaml`，产出 `ghcr.io/imleoo/kiro-rs:beta` | 服务器零编译负担；但当前 `ghcr.io/imleoo/kiro-rs` 包**尚不存在或非公开**（实测拉 tag 列表返回 `DENIED`），需先跑一次 workflow 并把 package 设为 public，或在服务器 `docker login ghcr.io` |
| C. 下载 Release 二进制 + `Dockerfile.release` | — | 本仓库目前无自建 Release 产物，不适用 |

**本方案按 A 展开**，B 作为后续 CI 化的升级路径（第六节）。

---

## 四、部署步骤

### 4.1 准备目录与源码

```bash
ssh root@172.104.94.251

mkdir -p /opt/kiro-rs && cd /opt/kiro-rs
git clone https://github.com/imleoo/kiro.rs-admin.git app
cd app
git log --oneline -1        # 确认拿到的是本 fork 的 master
```

### 4.2 写生产用 compose 文件

仓库自带的 `docker-compose.yml` 不改动（避免污染 git 工作区），新建 `/opt/kiro-rs/app/docker-compose.prod.yml`：

```yaml
services:
  kiro-rs:
    build:
      context: .
      dockerfile: Dockerfile
    image: kiro-rs-admin:local
    container_name: kiro-rs
    extra_hosts:
      - "host.docker.internal:host-gateway"
    ports:
      # 只绑回环：公网入口交给宿主 nginx，避免 Docker 绕过 UFW
      - "127.0.0.1:8990:8990"
    volumes:
      - /opt/kiro-rs/data:/app/config
      # 修正容器内时区：用量统计按 Local::now() 的自然日切分，
      # 不挂载则容器内为 UTC，日报表会与北京时间错开 8 小时
      - /usr/share/zoneinfo/Asia/Shanghai:/etc/localtime:ro
      # alpine 镜像不带 tzdata。只挂 /etc/localtime 不够：TZ=Asia/Shanghai 这种
      # 地区名需要查 /usr/share/zoneinfo，musl 查不到会静默回落 UTC 并覆盖
      # /etc/localtime 的效果，因此必须把 zoneinfo 一并挂进去（实测踩过）
      - /usr/share/zoneinfo:/usr/share/zoneinfo:ro
    environment:
      - TZ=Asia/Shanghai
      - RUST_LOG=info
    restart: unless-stopped
    logging:
      driver: json-file
      options:
        max-size: "50m"
        max-file: "5"
```

要点说明：

- `/app/config` 是**唯一**需要持久化的路径。程序把「凭据文件所在目录」当作数据目录
  （`TokenManager::cache_dir()`，`src/kiro/token_manager.rs:2450`），因此下列文件全部落在这里：
  `config.json`、`credentials.json`、`client_api_keys.json`、`proxy_pool.json`、`kiro_stats.json`、
  `kiro_balance_cache.json`、`usage_stats.json`、`usage_log.*.jsonl`、`cache_metering.json`、
  `model_mappings.json`、`groups.json`、`traces.db`(+`-wal`/`-shm`)。
- `extra_hosts` 保留：若日后想让某条凭据走宿主机上的 hysteria/本地代理，可在代理池里填
  `http://host.docker.internal:<port>`。

### 4.3 构建并首次启动

```bash
cd /opt/kiro-rs/app
docker compose -f docker-compose.prod.yml build      # 首次较慢，耐心等
docker compose -f docker-compose.prod.yml up -d
docker compose -f docker-compose.prod.yml logs -f kiro-rs
```

**不要手动 `cp config.example.json data/config.json`**——示例文件里 `"host": "127.0.0.1"`，
在容器内会导致端口映射不通。让程序自己生成：首次启动检测到 `/app/config/config.json` 不存在时，
会写入 `"host": "0.0.0.0"` 的默认配置，并在日志里打印随机生成的两把 Key
（`src/main.rs:426` `ensure_config_files`）：

```
已生成默认配置: /app/config/config.json
  apiKey      = sk-kiro-rs-xxxxxxxx（每次启动时同步为系统 Key）
  adminApiKey = sk-admin-xxxxxxxx（管理面板登录密钥）
```

**立刻把这两把 Key 记录到密码管理器**，随后可按需编辑 `/opt/kiro-rs/data/config.json` 补齐：

```jsonc
{
  "host": "0.0.0.0",
  "port": 8990,
  "apiKey": "sk-kiro-rs-...",
  "adminApiKey": "sk-admin-...",
  "region": "us-east-1",
  "tlsBackend": "rustls",          // 镜像用 --no-default-features 构建，只有 rustls，勿改 native-tls
  "defaultEndpoint": "ide",
  "updateAutoApply": false,        // ← 风险 2，必须保持 false
  "traceEnabled": true,
  "traceRetentionDays": 7,
  "usageLogRetentionDays": 31
}
```

改完重启：`docker compose -f docker-compose.prod.yml restart`

### 4.4 本机连通性自检

```bash
# 401 = 服务活着且鉴权生效（未带 Key）
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8990/v1/models
# 200 = 带 Key 正常
curl -s -H "x-api-key: sk-kiro-rs-..." http://127.0.0.1:8990/v1/models | head -c 300
# Admin UI
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8990/admin
```

---

## 五、公网入口：宝塔 nginx 反向代理

### 5.1 域名

使用 `kiro.leoobai.cn`（已解析到 `172.104.94.251`，服务器侧 `getent hosts` 验证通过）。

站点由宝塔面板管理，需要在面板完成两步（面板操作无法由脚本代劳，证书续期也依赖面板的站点记录）：

1. **网站 → 添加站点**：域名填 `kiro.leoobai.cn`，PHP 版本选「纯静态」，**不要**创建数据库/FTP；
2. **该站点 → SSL → Let's Encrypt**：勾选域名申请证书，并打开「强制 HTTPS」。

反代配置已**预置**在
`/www/server/panel/vhost/nginx/extension/kiro.leoobai.cn/proxy.conf`，
宝塔生成的 vhost 顶部有 `include .../extension/kiro.leoobai.cn/*.conf;`，
建站后会自动生效，无需再手工改 vhost；且面板后续修改站点配置也不会覆盖该文件。

### 5.2 关键：SSE 流式必须关缓冲

Anthropic `/v1/messages` 与 OpenAI `/v1/chat/completions` 都是 SSE 流式响应。
nginx 默认 `proxy_buffering on` 会把流缓存起来批量下发，表现为"客户端长时间无输出、最后一次性吐出"
甚至超时中断。**以下三项缺一不可**：

实际落地的 `extension/kiro.leoobai.cn/proxy.conf` 全文：

```nginx
# ACME 验证目录必须留给静态文件，否则证书续期会被代理吞掉
location ^~ /.well-known/ {
    root /www/wwwroot/kiro.leoobai.cn;
    try_files $uri =404;
}

# 其余全部反代。这里必须用 ^~ 而不是普通的 location /：
# 宝塔 vhost 自带 `location ~ .*\.(js|css)?$` 和图片正则规则，
# 正则 location 优先级高于普通前缀匹配，会把 Admin UI 的
# /admin/assets/*.js|css 当成本地静态文件处理导致 404。
# ^~ 命中后 nginx 不再检查正则，问题消除。
location ^~ / {
    proxy_pass http://127.0.0.1:8990;
    proxy_http_version 1.1;

    proxy_set_header Host              $host;
    proxy_set_header X-Real-IP         $remote_addr;
    proxy_set_header X-Forwarded-For   $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_set_header Connection        "";

    # —— SSE 流式三件套 ——
    proxy_buffering    off;
    proxy_cache        off;
    chunked_transfer_encoding on;

    # 长连接超时：单次推理可能跑很久
    proxy_read_timeout    3600s;
    proxy_send_timeout    3600s;
    proxy_connect_timeout 60s;

    # 请求体上限：多模态图片输入可能较大
    client_max_body_size 100m;
}
```

两个 location 的顺序无所谓，nginx 按「最长前缀」选中 `^~` 规则：
`/.well-known/acme-challenge/xxx` 命中前者走静态，其余命中后者走代理。

宝塔站点默认会带 `0.fastcgi_cache.conf` 之类的全局缓存配置，创建站点时**关闭 "缓存"** 相关开关。

### 5.3 Admin 面板访问收敛（建议）

`/admin` 与 `/api/admin/*` 有独立的 `adminApiKey` 鉴权，但仍建议在 nginx 层加 IP 白名单，
把管理面收敛到你的固定出口 IP：

```nginx
location /admin {
    allow  <你的固定 IP>;
    deny   all;
    proxy_pass http://127.0.0.1:8990;
    # ……其余 proxy_set_header 同上
}
```

若出口 IP 不固定，可退而求其次：不开放 `/admin`，需要时通过 SSH 隧道访问

```bash
ssh -L 8990:127.0.0.1:8990 root@172.104.94.251
# 然后本地浏览器打开 http://127.0.0.1:8990/admin
```

### 5.4 无需改动 UFW

因为容器只绑 `127.0.0.1:8990`，公网流量走已放行的 443。**不要**执行 `ufw allow 8990`。

### 5.5 踩坑：Let's Encrypt 验证失败 `Server is speaking HTTP/2 over HTTP`

首次在宝塔申请证书时报：

```
Fetching http://kiro.leoobai.cn/.well-known/acme-challenge/xxx:
Server is speaking HTTP/2 over HTTP
```

**根因（与本项目无关，是服务器既有的 nginx 配置错误）**：

- 实际运行的是宝塔编译的 Tengine（`/www/server/nginx/sbin/nginx`，支持 1.25+ 的 `http2 on` 指令），
  不是 `/usr/sbin/nginx` 那个系统包 1.24.0；
- `0.default.conf` 是 80 端口的**默认 server**（`server_name _`），里面同时写了 `listen 80;` 与 `http2 on;`；
- 明文连接的 h2c 判定取默认 server 的配置，于是**整个 80 端口**对所有站点都主动下发 HTTP/2 SETTINGS 帧。
  用 `nc` 抓原始响应可见开头是 `\0\0\x12\x04`（HTTP/2 SETTINGS 帧）而非 `HTTP/1.1 200`。

该问题**先于本次部署就存在**：实测 `ai.leoobai.cn` 的 80 端口返回同样的二进制帧，
即所有站点的 HTTP→HTTPS 跳转对 HTTP/1.1 客户端都是坏的，只是平时都直接走 HTTPS 没被发现。

**修复**（2026-08-03 已执行，改前已备份为 `0.default.conf.bak-<时间戳>`）：

```bash
# 删掉默认站点的 http2 on
sed -i '/^    http2 on;$/d' /www/server/panel/vhost/nginx/0.default.conf
# 注意用宝塔的 nginx 二进制测试与 reload，不是 /usr/sbin/nginx
/www/server/nginx/sbin/nginx -c /www/server/nginx/conf/nginx.conf -t
/www/server/nginx/sbin/nginx -c /www/server/nginx/conf/nginx.conf -s reload
```

修复后回归验证：80 端口恢复 `HTTP/1.1 301`/`404` 明文响应；
`jp.b517.com`、`openclaw.zhiguo.fan` 等站点 443 仍为 `http/2`（各自 server 块自带 http2，不受影响）。

> `ai.leoobai.cn`、`jb2api.zhiguo.fan` 的 443 返回 502 与本次改动无关——
> 它们分别代理到未启动的 `127.0.0.1:3000`、`127.0.0.1:8742`，错误日志在改动前数小时就已在报
> `Connection refused`。

**自检方法**（申请证书前先跑一遍，避免反复失败）：

```bash
W=/www/wwwroot/kiro.leoobai.cn
mkdir -p $W/.well-known/acme-challenge
echo ok > $W/.well-known/acme-challenge/selftest
curl -s http://kiro.leoobai.cn/.well-known/acme-challenge/selftest   # 应输出 ok
rm -f $W/.well-known/acme-challenge/selftest
```

### 5.6 踩坑：Admin UI 静态资源 404

证书装好后打开 `https://kiro.leoobai.cn/admin`，HTML 返回 200 但控制台刷屏：

```
GET /admin/assets/index-DoemGbI3.js  404 (Not Found)
GET /admin/assets/index-DcQelM1K.css 404 (Not Found)
...
```

**根因**：宝塔生成的 vhost 里有两条静态资源正则规则

```nginx
location ~ .*\.(gif|jpg|jpeg|png|bmp|swf)$ { expires 30d; }
location ~ .*\.(js|css)?$                  { expires 12h; }
```

nginx 的 location 匹配规则中**正则优先于普通前缀匹配**，所以 `/admin/assets/*.js` 命中的是
上面这条正则而不是我们的 `location /`。该 location 内没有 `proxy_pass`，于是继承 server 的
`root /www/wwwroot/kiro.leoobai.cn` 去找本地文件——而 Admin UI 是用 rust-embed 编译进二进制的，
磁盘上根本没有这些文件，必然 404。

**修复**：把代理 location 从 `location /` 改成 `location ^~ /`。`^~` 命中后 nginx 不再检查正则，
静态资源随之走代理。同时补一条 `location ^~ /.well-known/`（更长前缀，优先级更高）把 ACME
验证目录留给静态文件，避免证书续期被代理吞掉。配置见 5.2。

验证结果：`/admin/assets/*.js` 返回 200 且 `content-type: text/javascript`；
`/admin/assets/*.css` 返回 200 `text/css`；ACME 自检文件仍能读到。

---

## 六、运维

### 6.1 更新（唯一正确路径）

```bash
cd /opt/kiro-rs/app
git pull
docker compose -f docker-compose.prod.yml build
docker compose -f docker-compose.prod.yml up -d      # 重建容器，data/ 不受影响
docker image prune -f
```

回滚：`git checkout <上一个 commit>` 后重复 build/up；或保留上一版镜像 tag
（`docker tag kiro-rs-admin:local kiro-rs-admin:$(date +%Y%m%d)` 在每次 build 前执行）。

**再次强调：不要用 Admin 面板的在线更新**（风险 2）。

### 6.2 备份

`/opt/kiro-rs/data/` 含明文 Kiro 刷新令牌与客户端 Key，**属于敏感数据**（目录已 `chmod 700`）。

已部署脚本 `/opt/kiro-rs/backup.sh`，并写入 crontab **每日 04:30** 执行、保留 14 天：

```bash
#!/bin/bash
set -euo pipefail
DEST=/root/backup/kiro
tar czf "$DEST/kiro-data-$(date +%F).tgz" -C /opt/kiro-rs data
chmod 600 "$DEST/kiro-data-$(date +%F).tgz"
find "$DEST" -name "kiro-data-*.tgz" -mtime +14 -delete
```

手动执行一次：`/opt/kiro-rs/backup.sh && ls -lh /root/backup/kiro/`

`traces.db` 是 WAL 模式的 SQLite，热备可能拿到不一致快照。若要精确备份 trace，先停容器再打包；
但 trace 属于可丢弃的可观测数据，日常备份只保 `config.json` / `credentials.json` /
`client_api_keys.json` / `proxy_pool.json` / `model_mappings.json` 也够。

目录权限收紧：`chmod 700 /opt/kiro-rs/data`

### 6.3 日志

compose 已配 json-file 轮转（50 MB × 5）。查看：

```bash
docker compose -f docker-compose.prod.yml logs --tail 200 -f
```

排障时临时开 debug：把 `RUST_LOG=info` 改为 `RUST_LOG=debug` 再 `up -d`（注意 debug 会打印较多上游交互细节）。

### 6.4 可观测

- Admin 面板「请求日志」页有完整重试链路（凭据 / 端点 / HTTP 状态 / 失败分类 / 各阶段耗时）；
- `traceRetentionDays: 7` 控制保留期，磁盘充裕可调大；
- 容器资源：`docker stats kiro-rs`。

---

## 七、验证清单（2026-08-03 部署实测结果）

| # | 检查项 | 结果 |
|---|---|---|
| 1 | `docker ps` 中 `kiro-rs` 为 `Up`，重启策略 `unless-stopped` | ✅ |
| 2 | `ss -ltnp \| grep 8990` 监听在 `127.0.0.1` 而非 `0.0.0.0` | ✅ `127.0.0.1:8990->8990/tcp` |
| 3 | 公网 `http://172.104.94.251:8990` 不可达 | ✅ 连接失败（curl `000`） |
| 4 | `curl 127.0.0.1:8990/v1/models` 无 Key | ✅ 401 |
| 5 | `curl 127.0.0.1:8990/admin` | ✅ 200 |
| 6 | 跑的是本 fork 而非上游镜像 | ✅ 启动日志含 `已注册端点桶: [amazonq, cli, codewhisperer, ide, runtime, runtime_cli]`（上游仅 `ide`/`cli`）与 `账号级风控转移: 开启` |
| 7 | `data/config.json` 中 `updateAutoApply` 为 `false` | ✅ 已显式写入 |
| 8 | 容器内时区为北京时间 | ✅ `CST`（首次配置踩坑，见 4.2 注释） |
| 9 | `docker compose restart` 后数据仍在 | ✅ `config.json`/`credentials.json`/`client_api_keys.json`/`model_mappings.json`/`traces.db` 均保留 |
| 10 | 备份 cron 已配置且首次执行成功 | ✅ `/root/backup/kiro/kiro-data-2026-08-03.tgz` |
| 11 | `nginx -t` 通过 | ✅ |
| 12 | 域名解析 | ✅ 服务器侧 `kiro.leoobai.cn → 172.104.94.251` |

**公网入口验证（证书装好后补测）：**

| # | 检查项 | 结果 |
|---|---|---|
| 13 | 宝塔添加站点 + Let's Encrypt 证书 + 强制 HTTPS | ✅（过程中修掉 5.5 的 h2c 问题） |
| 14 | `https://kiro.leoobai.cn/admin` | ✅ 200 |
| 15 | Admin UI 静态资源 `/admin/assets/*.js\|css` | ✅ 200，MIME 正确（见 5.6） |
| 16 | `https://kiro.leoobai.cn/v1/models` 无 Key | ✅ 401 |
| 17 | HTTP 强制跳 HTTPS | ✅ `301 → https://kiro.leoobai.cn/admin` |
| 18 | ACME 目录未被代理吞掉（续期可用） | ✅ 自检文件可读 |

**遗留待办：**

- [ ] Admin 面板添加至少一条 Kiro 凭据，「凭据测试」能拿到响应
      （在此之前 `/v1/models` 带 Key 返回 503 属正常——凭据数为 0，模型列表拉不到）
- [ ] 用真实客户端跑一次**流式**请求，确认 token 逐步吐出而非一次性返回（验证 nginx 未缓冲）
- [ ] 视情况给 `/admin` 加 IP 白名单（5.3）

---

## 八、已知待办 / 后续优化（非本次必需）

1. **在线更新指向 fork**：修改 `binary_update.rs:26` / `service.rs:580` 的 `GITHUB_REPO`
   为 `imleoo/kiro.rs-admin`，并给本仓库打 tag 产出 Release，才能安全使用面板内更新。
2. **Dockerfile 依赖层缓存**：现在 `COPY src` 后直接 `cargo build`，任何代码改动都触发全量重编译。
   引入 cargo-chef 或"先 COPY Cargo.toml + 假 main.rs 预编译依赖"可把增量构建从十几分钟压到分钟级。
3. **CI 化**（方案 B）：跑一次 `docker-build.yaml`（push master 或 workflow_dispatch）产出
   `ghcr.io/imleoo/kiro-rs:beta`，把 GHCR package 设为 public，服务器改为 `image:` + `docker compose pull`，
   服务器不再承担编译。
4. **健康检查**：项目未暴露 `/health` 端点，`/v1/models` 需鉴权（返回 401）。如需 compose healthcheck，
   用 TCP 探测（`nc -z 127.0.0.1 8990`）而非 HTTP 200 判断。
