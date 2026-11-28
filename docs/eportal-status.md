# Dr.COM 校园网状态查询接口

| 接口 | 方法 | 地址 | 用途 |
| --- | --- | --- | --- |
| `chkstatus` | GET | `http://172.18.254.6/drcom/chkstatus` | 查询本机认证状态与接入信息 |
| `online_list` | GET | `http://172.18.254.6:803/eportal/portal/online_list` | 查询门户会话下的在线列表 |

本机在线状态统一以 `chkstatus` 为准；终端已在线时，`online_list` 仍可能返回空列表。

## `chkstatus`

### 请求参数

| 参数 | 说明 |
| --- | --- |
| `callback` | JSONP 回调名，客户端使用 `dr1` |
| `v` | 防缓存值，客户端使用 Unix 毫秒时间戳 |

```http
GET http://172.18.254.6/drcom/chkstatus?callback=dr1&v=1234
```

### 响应字段

`result=1` 表示认证在线，`result=0` 表示离线：

```javascript
dr1({"result":1,"v46ip":"10.0.0.100","vid":0,"myv6ip":"::","uid":"2026000000"})
dr1({"result":0,"v46ip":"10.0.0.100","vid":0,"myv6ip":""})
```

客户端按下列优先级提取接入信息（数值字段转换为字符串）：

| 信息 | 候选字段（按优先级） | 缺失处理 |
| --- | --- | --- |
| IPv4 | `v46ip`、`v4ip`、`userip`、`user_ip`、`wlan_user_ip` | 返回 `MissingIp` |
| IPv6 | `wlan_user_ipv6`、`myv6ip` | 空字符串 |
| MAC | `wlan_user_mac`、`mac`、`ss4` | 空字符串（登录时使用全零占位） |
| VLAN | `wlan_vlan_id`、`vlan`、`vid` | `1` |
| AC IP | `wlan_ac_ip`、`ac_ip` | 登录与注销传空 |
| AC 名称 | `wlan_ac_name`、`acname` | 登录与注销传空 |

IPv4 按顺序解析候选字段，取第一个合法地址。跳过无法解析为 IPv4、`0.0.0.0`、`255.255.255.255` 和组播地址。所有候选均无效时返回 `MissingIp`。

### 解析规则

- 响应支持匹配回调名的 JSONP 与裸 JSON 对象。
- 在线状态按 `online` → `is_login` → `result` 的优先级读取，命中首个存在的字段即停止：`online` 与 `is_login` 接受布尔值及数字/字符串 `0`/`1`，`result` 接受数字/字符串 `0`/`1`；字段缺失或值无法识别时映射为 `Unknown`。

## `online_list`

门户页面在 `check_online_method=1` 或 `port_mode=1` 时调用此接口，调用前需通过 [`loadConfig`](./eportal-login.md) 获取 `program_index` 与 `page_index`。浏览器请求会附带首页 Referer 与 `PHPSESSID` Cookie，服务端是否强制校验待确认。

### 请求参数

| 参数 | 说明 |
| --- | --- |
| `login_method` | `1` |
| `user_account`、`user_password`、`uuid` | 本机查询传空 |
| `wlan_user_ip`、`wlan_user_ipv6`、`wlan_ac_ip` | 先经标准 Base64 编码，再做 URL 编码；缺失传空 |
| `wlan_user_mac` | 终端 MAC，缺失传 `000000000000` |
| `program_index`、`page_index` | `loadConfig` 返回的索引 |
| `callback`、`jsVersion`、`v`、`lang` | 回调名、`4.X`、防缓存随机数、`zh` |

```http
GET http://172.18.254.6:803/eportal/portal/online_list?callback=dr1002&user_account=&user_password=&wlan_user_mac=000000000000&wlan_user_ip=MTAuMC4wLjEwMA%3D%3D&wlan_user_ipv6=&jsVersion=4.X&uuid=&login_method=1&wlan_ac_ip=&program_index=example-program&page_index=example-page&v=1234&lang=zh
```

### 响应字段

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `result` | 整数 | `1` 表示获取到在线记录，`0` 表示未获取到 |
| `msg` | 字符串 | 门户提示信息 |
| `list` | 数组 | 在线记录列表（在线时返回 1 条，条目完整字段待补充） |
| `total` | 整数 | 记录总数 |

```javascript
dr1002({"result":0,"msg":"获取用户在线信息数据为空！","list":[],"total":0})
```

## 客户端行为

`Client::status()` 直连 `chkstatus`，禁用代理与自动重定向；连接超时 3 秒，请求超时 8 秒，响应正文上限 64 KiB。后台仅在连接目标 Wi-Fi 且门户状态明确为 `Offline` 时触发认证。实现见 [client.rs](../crates/wlan-api/src/client.rs) 与 [policy.rs](../crates/wlan-app/src/app/daemon/policy.rs)。
