# Dr.COM 校园网状态查询接口

| 接口 | 方法 | 地址 | 用途 |
| --- | --- | --- | --- |
| `chkstatus` | GET | `http://172.18.254.6/drcom/chkstatus` | 本机认证状态及接入信息 |
| `online_list` | GET | `http://172.18.254.6:803/eportal/portal/online_list` | 门户会话下的在线记录 |

用 `chkstatus` 判断本机是否已经认证。`online_list` 的空列表也可能出现在本机在线时。

## chkstatus

### 请求参数

| 参数 | 取值 |
| --- | --- |
| `callback` | JSONP 回调名。客户端使用 `dr1` |
| `v` | 防缓存值。客户端使用 Unix 毫秒时间戳 |

```http
GET http://172.18.254.6/drcom/chkstatus?callback=dr1&v=1234
```

### 响应字段

JSONP 正文中，`result=1` 表示认证在线，`result=0` 表示离线。判断认证状态时读取正文的业务字段。下列示例使用虚构的终端 IP 和账号。

```javascript
dr1({"result":1,"v46ip":"10.0.0.100","vid":0,"myv6ip":"::","uid":"2026000000"})
dr1({"result":0,"v46ip":"10.0.0.100","vid":0,"myv6ip":""})
```

当前客户端按下表所列顺序读取接入信息。数值型接入字段会转换为字符串。

| 信息 | 字段名（按优先级） | 缺失时的处理 |
| --- | --- | --- |
| 设备 IPv4 | `v46ip`、`v4ip`、`userip`、`user_ip`、`wlan_user_ip` | 返回 `MissingIp` |
| 设备 IPv6 | `wlan_user_ipv6`、`myv6ip` | 空字符串 |
| 设备 MAC | `wlan_user_mac`、`mac`、`ss4` | 空字符串。提交登录时使用全零占位值 |
| VLAN | `wlan_vlan_id`、`vlan`、`vid` | `1` |
| 接入控制器 IP | `wlan_ac_ip`、`ac_ip` | 登录和注销传空 |
| 接入控制器名称 | `wlan_ac_name`、`acname` | 登录和注销传空 |

客户端校验选中的 IPv4 字段。地址无效、为 `0.0.0.0`、为 `255.255.255.255` 或为组播地址时返回 `MissingIp`，即使正文提供了认证状态。

### 客户端兼容规则

认证状态按 `online`、`is_login`、`result` 的优先级读取。前两者接受布尔值及数字或字符串 `0`、`1`。`result` 接受数字 `0`、`1` 及对应的字符串。缺失或无法识别的状态映射为 `Unknown`。客户端优先使用存在的字段，遇到无法识别的值即返回 `Unknown`。

客户端接受匹配请求回调名的 JSONP 和裸 JSON 对象。解析只读取 JSON 内容。

## online_list

门户页面在 `check_online_method=1` 或 `port_mode=1` 时调用此端点。先加载门户配置，再使用返回的方案和页面索引。[页面配置接口](./eportal-login.md)

### 请求参数

| 参数 | 取值 |
| --- | --- |
| `login_method` | 本校 Portal 认证方式为 `1` |
| `user_account`、`user_password`、`uuid` | 本机状态查询传空 |
| `wlan_user_ip`、`wlan_user_ipv6`、`wlan_ac_ip` | 地址文本先做标准 Base64，再做 URL 编码。缺失的地址传空 |
| `wlan_user_mac` | 终端 MAC。缺失时传 `000000000000` |
| `program_index`、`page_index` | 页面配置返回的索引 |
| `callback`、`jsVersion`、`v`、`lang` | 回调名、`4.X`、防缓存值、`zh` |

浏览器携带门户首页 Referer 和当前 `PHPSESSID` 会话 Cookie。服务端是否要求这些请求头尚未确定。

```http
GET http://172.18.254.6:803/eportal/portal/online_list?callback=dr1002&user_account=&user_password=&wlan_user_mac=000000000000&wlan_user_ip=MTAuMC4wLjEwMA%3D%3D&wlan_user_ipv6=&jsVersion=4.X&uuid=&login_method=1&wlan_ac_ip=&program_index=example-program&page_index=example-page&v=1234&lang=zh
```

### 响应字段

| 字段 | 示例类型 | 含义 |
| --- | --- | --- |
| `result` | 整数 | `1` 表示门户获得在线信息，`0` 可表示本次查询未取得记录 |
| `msg` | 字符串 | 门户提示 |
| `list` | 数组 | 在线记录 |
| `total` | 整数 | 记录总数 |

```javascript
dr1002({"result":0,"msg":"获取用户在线信息数据为空！","list":[],"total":0})
```

在线时可以返回 `result=1`、`total=1` 和一条列表记录。列表记录的完整字段定义待确认。调用方需要确认本机离线时，读取 `chkstatus.result=0`。

## 客户端调用约定

`Client::status()` 使用 `chkstatus`。登录和注销都先查询该接口取得终端信息。门户接受注销后，客户端再次查询并确认同一终端 IPv4 的状态为 `Offline`。后台仅在目标 Wi-Fi 会话仍有效且状态为 `Offline` 时尝试认证。

客户端直接连接门户，禁用应用层代理和自动重定向。连接超时为 3 秒，单次请求默认超时为 8 秒，响应正文上限为 64 KiB。失败会保留请求阶段。首次接入的临时状态查询错误使用 `1`、`2`、`4` 秒快速重试，常规失败退避依次为 `30`、`60`、`120` 秒。在线轮询间隔为 4 分钟，登录成功后的首次复查间隔为 30 秒。[请求实现](../crates/wlan-api/src/client.rs)、[后台策略](../crates/wlan-app/src/app/daemon/policy.rs)
