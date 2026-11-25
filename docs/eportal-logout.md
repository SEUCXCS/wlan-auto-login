# ePortal 校园网注销接口

注销用于结束当前终端的门户认证会话。`Client::logout()` 使用端口 `803` 的新版接口。注销前先通过 `chkstatus` 取得本机有效 IPv4，再以在线页参数加载门户配置。[状态查询接口](./eportal-status.md)

## 新版注销

```http
GET http://172.18.254.6:803/eportal/portal/logout
```

### 请求参数

| 参数 | 取值 |
| --- | --- |
| `login_method` | 本校 Portal 认证使用 `1` |
| `user_account`、`user_password` | 协议占位值 `drcom`、`123` |
| `ac_logout` | 页面配置指定的注销方式。门户脚本定义 `0` 为默认方式，`1` 为 RADIUS，`2` 为全业务接口。本校配置使用 `1` |
| `register_mode` | 页面配置指定的后台类型。本校配置使用 `1` |
| `wlan_user_ip`、`wlan_user_ipv6` | 当前终端的地址，以文本传递 |
| `wlan_vlan_id` | 当前在线页上下文的 VLAN |
| `wlan_user_mac` | 终端 MAC。浏览器可传全零占位值 `000000000000` |
| `wlan_ac_ip`、`wlan_ac_name` | 接入控制器 IP 和名称。无对应值时可传空 |
| `program_index`、`page_index` | `page/loadConfig` 返回的方案、页面索引 |
| `callback`、`jsVersion`、`v`、`lang` | 回调名、`4.5.1`、防缓存值、`zh` |

浏览器的在线页可使用 VLAN `1` 和 IPv6 `::`，即使登录页的 VLAN 为 `0`、IPv6 为空。示例采用在线页的取值，替换了设备 IPv4、页面索引和防缓存值。

```http
GET http://172.18.254.6:803/eportal/portal/logout?callback=dr1002&login_method=1&user_account=drcom&user_password=123&ac_logout=1&register_mode=1&wlan_user_ip=10.0.0.100&wlan_user_ipv6=%3A%3A&wlan_vlan_id=1&wlan_user_mac=000000000000&wlan_ac_ip=&wlan_ac_name=&jsVersion=4.5.1&program_index=example-program&page_index=example-page&v=1234&lang=zh
```

浏览器携带门户首页 Referer 和当前 `PHPSESSID` 会话 Cookie。这些请求头是否必需、服务端接受的最小查询参数集合均待确认。

客户端使用同一组终端参数加载配置和提交注销。按本校在线页的取值，状态 VLAN 为 `0` 时，注销使用 `1`，缺失的 IPv6 使用 `::`。其他 VLAN 和 IPv6 值取自本机状态。配置中的 `ac_logout`、`register_mode` 缺失时使用页面脚本的默认值 `0`。

### 响应字段

| 字段 | 示例类型 | 含义 |
| --- | --- | --- |
| `result` | 整数 | `1` 表示注销成功，`0` 表示本次注销失败 |
| `msg` | 字符串 | 门户提示。失败提示可同时列出未在线、页面过期等可能原因 |

```javascript
dr1002({"result":1,"msg":"Radius注销成功！"})
dr1002({"result":0,"msg":"注销失败，可能不在线或者页面已过期，请刷新页面重试！"})
```

以正文 `result` 判断操作结果。客户端也接受 `result` 为 `"1"` 或 `"ok"` 的成功响应。随后以 `chkstatus` 确认同一终端 IPv4 已离线。相同请求参数可能得到成功或失败结果，通用提示只能用来报告失败。`online_list` 空列表也可能出现在本机在线时。[状态查询接口](./eportal-status.md)

## 旧版注销

```http
GET http://172.18.254.6:801/eportal/?c=Portal&a=logout
```

| 参数 | 旧版客户端传值 |
| --- | --- |
| `c`、`a` | `Portal`、`logout` |
| `login_method` | `1` |
| `user_account`、`user_password` | `drcom`、`123` |
| `ac_logout`、`register_mode` | `0`、`1` |
| `wlan_user_ip`、`wlan_user_ipv6`、`wlan_user_mac`、`wlan_vlan_id` | 来自 `chkstatus` |
| `wlan_ac_ip`、`wlan_ac_name` | 优先使用 `chkstatus` 的值，缺失时默认 `172.18.255.6`、`NAS` |
| `callback`、`jsVersion`、`v` | `dr1`、`3.3.3`、Unix 毫秒时间戳 |

```http
GET http://172.18.254.6:801/eportal/?c=Portal&a=logout&callback=dr1&login_method=1&user_account=drcom&user_password=123&ac_logout=0&register_mode=1&wlan_user_ip=10.0.0.100&wlan_user_ipv6=&wlan_vlan_id=1&wlan_user_mac=001122334455&wlan_ac_ip=172.18.255.6&wlan_ac_name=NAS&jsVersion=3.3.3&v=1234
```

旧版页面在 `login_method=0` 时另有 `/drcom/logout` 分支。页面元数据中还列有 `/eportal/?c=ACSetting&a=Logout&ver=1.0` 作为 Dr.COM 客户端的注销路径。

## 客户端调用约定

CLI 的 `logout` 暂停当前连接的后台自动认证。初次查询已确认离线时，命令直接完成。若门户接受注销但复查未能确认离线，命令报告下线未确认，后台保持暂停。`logout --forget` 另外停止相应作用域的后台进程并删除本地保存的凭据。远端注销失败时，`--forget` 仍尝试本地清理，并报告远端操作失败。[登录接口](./eportal-login.md)、[客户端实现](../crates/wlan-api/src/client.rs)
