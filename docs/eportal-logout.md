# ePortal 校园网注销接口

`Client::logout()` 使用端口 `803` 的新版注销接口。注销前先通过 [`chkstatus`](./eportal-status.md) 获取本机 IPv4，再加载页面配置取得方案与页面索引。

## 新版注销

```http
GET http://172.18.254.6:803/eportal/portal/logout
```

### 请求参数

| 参数 | 说明 |
| --- | --- |
| `login_method` | `1`（Portal 认证） |
| `user_account`、`user_password` | 协议固定占位值 `drcom`、`123` |
| `ac_logout` | 注销方式，取自页面配置（`0` 默认、`1` RADIUS、`2` 全业务接口；本校为 `1`，缺失默认 `0`） |
| `register_mode` | 后台类型，取自页面配置（本校为 `1`，缺失默认 `0`） |
| `wlan_user_ip`、`wlan_user_ipv6` | 终端地址明文 |
| `wlan_vlan_id` | 在线页上下文的 VLAN |
| `wlan_user_mac` | 终端 MAC，可使用全零占位值 `000000000000` |
| `wlan_ac_ip`、`wlan_ac_name` | 接入控制器 IP 与名称，缺失传空 |
| `program_index`、`page_index` | [`loadConfig`](./eportal-login.md) 返回的索引 |
| `callback`、`jsVersion`、`v`、`lang` | 回调名（客户端发 `dr1`，兼容 `dr0001`）、`4.5.1`、防缓存随机数、`zh` |

在线页发起注销时，即使登录页上下文的 VLAN 为 `0`、IPv6 为空，浏览器仍会传入 `wlan_vlan_id=1` 与 `wlan_user_ipv6=::`。客户端沿用该参数约定：状态查询返回 VLAN `0` 时改传 `1`，IPv6 缺失时改传 `::`，加载配置与提交注销使用同一组参数。

浏览器请求同时携带门户首页 Referer 与 `PHPSESSID` Cookie，服务端是否强制校验待确认。

```http
GET http://172.18.254.6:803/eportal/portal/logout?callback=dr1002&login_method=1&user_account=drcom&user_password=123&ac_logout=1&register_mode=1&wlan_user_ip=10.0.0.100&wlan_user_ipv6=%3A%3A&wlan_vlan_id=1&wlan_user_mac=000000000000&wlan_ac_ip=&wlan_ac_name=&jsVersion=4.5.1&program_index=example-program&page_index=example-page&v=1234&lang=zh
```

### 响应字段

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `result` | 整数 | `1` 表示成功（客户端兼容 `"1"` 与 `"ok"`），`0` 表示失败 |
| `msg` | 字符串 | 门户提示（失败时通常返回“可能不在线或页面已过期”等通用文案） |

```javascript
dr1002({"result":1,"msg":"Radius注销成功！"})
dr1002({"result":0,"msg":"注销失败，可能不在线或者页面已过期，请刷新页面重试！"})
```

收到成功响应后，客户端会再次调用 `chkstatus` 复查，确认同一 IPv4 已离线。

## 旧版注销（参考）

```http
GET http://172.18.254.6:801/eportal/?c=Portal&a=logout
```

| 参数 | 取值 |
| --- | --- |
| `c`、`a` | `Portal`、`logout` |
| `login_method` | `1` |
| `user_account`、`user_password` | `drcom`、`123` |
| `ac_logout`、`register_mode` | `0`、`1` |
| `wlan_user_ip`、`wlan_user_ipv6`、`wlan_user_mac`、`wlan_vlan_id` | 取自 `chkstatus` |
| `wlan_ac_ip`、`wlan_ac_name` | 优先取自 `chkstatus`，缺失默认 `172.18.255.6`、`NAS` |
| `callback`、`jsVersion`、`v` | `dr1`、`3.3.3`、Unix 毫秒时间戳 |

```http
GET http://172.18.254.6:801/eportal/?c=Portal&a=logout&callback=dr1&login_method=1&user_account=drcom&user_password=123&ac_logout=0&register_mode=1&wlan_user_ip=10.0.0.100&wlan_user_ipv6=&wlan_vlan_id=1&wlan_user_mac=001122334455&wlan_ac_ip=172.18.255.6&wlan_ac_name=NAS&jsVersion=3.3.3&v=1234
```

旧版页面在 `login_method=0` 时使用 `/drcom/logout`；页面元数据中另包含 Dr.COM 客户端注销路径 `/eportal/?c=ACSetting&a=Logout&ver=1.0`。

## 客户端流程

初次查询 `chkstatus` 已离线时，CLI `logout` 直接返回完成；门户接受注销但复查仍在线时报错提示下线未确认。无论上述哪种情况，当前连接的后台自动登录均会暂停。相关实现见 [登录接口](./eportal-login.md) 与 [client.rs](../crates/wlan-api/src/client.rs)。
