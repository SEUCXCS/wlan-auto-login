# ePortal 校园网登录接口

| 接口 | 方法 | 地址 |
| --- | --- | --- |
| 页面配置 | GET | `http://172.18.254.6:803/eportal/portal/page/loadConfig` |
| 账号登录 | GET | `http://172.18.254.6:803/eportal/portal/login` |

两者使用 URL 查询参数和 JSONP 响应。浏览器从当前门户会话发送 `PHPSESSID` 和首页 Referer。服务端对这些请求头及各查询参数的最小要求尚未确定。

## 页面配置

页面配置返回门户方案、页面索引和认证选项。先取得当前设备的 IPv4，再用终端接入信息加载配置。登录、注销和 `online_list` 请求使用返回的索引。[设备状态查询](./eportal-status.md)

### 请求参数

| 参数 | 取值 |
| --- | --- |
| `program_index`、`page_index` | 初次加载时均为空 |
| `wlan_vlan_id` | 当前页面上下文的 VLAN。客户端使用状态查询的值 |
| `wlan_user_ip`、`wlan_user_ipv6`、`wlan_ac_ip` | 地址文本先做标准 Base64，再做 URL 编码。缺失的 IPv6 和 AC IP 传空 |
| `wlan_user_ssid`、`wlan_user_areaid` | SSID、区域标识。无对应页面信息时传空 |
| `wlan_ap_mac`、`gw_id` | AP MAC、网关标识。无对应页面信息时传 `000000000000` |
| `callback` | JSONP 回调名。客户端使用 `dr1` |
| `jsVersion` | 页面配置初始化使用 `4.X`。后续账号登录请求使用 `4.5.1` |
| `v`、`lang` | 防缓存值、`zh` |

```http
GET http://172.18.254.6:803/eportal/portal/page/loadConfig?callback=dr1001&program_index=&page_index=&wlan_vlan_id=0&wlan_user_ip=MTAuMC4wLjEwMA%3D%3D&wlan_user_ipv6=&wlan_user_ssid=&wlan_user_areaid=&wlan_ac_ip=&wlan_ap_mac=000000000000&gw_id=000000000000&jsVersion=4.X&v=1234&lang=zh
```

### 响应字段

| 字段 | 示例类型 | 含义 |
| --- | --- | --- |
| `code` | 整数 | `1` 表示配置加载成功，`0` 表示失败 |
| `msg` | 字符串 | 门户提示信息 |
| `data.program_index`、`data.page_index` | 字符串 | 方案索引、页面索引 |
| `data.login_method` | 整数 | 认证方式。本校 Portal 账号认证使用 `1` |
| `data.account_prefix` | 字符串 | 账号前缀开关。本校启用 `,0,` 前缀 |
| `data.check_online_method` | 字符串 | 在线状态查询方式。为 `1` 时，浏览器调用 `online_list` |
| `data.ac_logout`、`data.register_mode` | 字符串 | 注销方式、后台类型。本校配置为 `1`、`1` |
| `data.enable_r3` | 整数 | PPPoE 代拨开关。缺失时客户端使用 `0` |
| `data.rcn` | 字符串 | 页面随机数。缺失时客户端使用空字符串 |

成功响应示例省略了与账号认证无关的页面配置。

```javascript
dr1001({
  "code": 1,
  "data": {
    "program_index": "example-program",
    "page_index": "example-page",
    "login_method": 1,
    "account_prefix": "1",
    "check_online_method": "1",
    "ac_logout": "1",
    "register_mode": "1"
  }
})
```

`Client::login()` 在每次提交登录前重新读取索引。配置查询失败、`code=0` 或缺少索引时，本次登录结束于配置阶段。

## 账号登录

### 请求参数

| 参数 | 取值 |
| --- | --- |
| `login_method`、`business_type` | `1`、`1`，对应 Portal 账号认证 |
| `operate`、`is_base64encode` | `portal_login`、`1` |
| `user_account` | 按下节格式构造账号后，使用标准 Base64 编码其 UTF-8 字节 |
| `user_password` | 使用标准 Base64 编码密码的 UTF-8 字节 |
| `wlan_user_ip`、`wlan_user_ipv6` | 终端地址按文本传递，缺失的 IPv6 传空 |
| `wlan_user_mac`、`wlan_vlan_id` | 终端 MAC 和 VLAN。客户端去掉 MAC 分隔符，缺失时使用 `000000000000` |
| `wlan_ac_ip`、`wlan_ac_name` | 接入控制器 IP 和名称，缺失时传空 |
| `program_index`、`page_index` | `page/loadConfig` 返回的索引 |
| `terminal_type`、`mac_type` | 桌面端使用 `1`、`0` |
| `user_agent` | 浏览器传自身 User-Agent，客户端传 `wlanlogin/<版本>` |
| `enable_r3`、`rcn` | 页面配置中的值 |
| `authex_enable`、`uuid` | 本校账号认证流程传空 |
| `callback`、`jsVersion`、`v` | JSONP 回调名、`4.5.1`、防缓存值 |
| `lang` | 保留浏览器使用的两个同名参数，依次为 `zh-cn`、`zh` |

将全部查询参数进行 URL 编码，包括 Base64 的填充符 `=`。示例使用虚构凭据和索引。

```http
GET http://172.18.254.6:803/eportal/portal/login?callback=dr1&login_method=1&is_base64encode=1&user_account=LDAsMjAyNjAwMDAwMEBkaWFueGlu&user_password=ZXhhbXBsZS1wYXNzd29yZA%3D%3D&wlan_user_ip=10.0.0.100&wlan_user_ipv6=&wlan_user_mac=000000000000&wlan_vlan_id=0&wlan_ac_ip=&wlan_ac_name=&authex_enable=&jsVersion=4.5.1&uuid=&terminal_type=1&lang=zh-cn&user_agent=wlanlogin%2F0.1.1&enable_r3=0&mac_type=0&rcn=&operate=portal_login&business_type=1&program_index=example-program&page_index=example-page&v=1234&lang=zh
```

### 账号格式与线路

启用账号前缀时，编码前的格式为 `,0,<学号><线路后缀>`。当前客户端默认使用前缀，配置明确返回 `account_prefix=0` 时省略 `,0,`。

| 客户端线路 | 后缀 |
| --- | --- |
| 临时免费 1（`free1`） | `@dianxin` |
| 免费 2（`free2`） | `@free` |
| 内网（`campus`，当前客户端映射） | `@xyw` |

Base64 可还原原始凭据。日志和抓包示例应遮盖整个 `user_account`、`user_password` 和会话 Cookie。

### 响应字段

| 字段 | 示例类型 | 含义 |
| --- | --- | --- |
| `result` | 整数 | `1` 表示本次认证成功，`0` 表示认证失败 |
| `msg` | 字符串 | 门户返回的提示 |
| `ret_code` | 整数 | 失败时可附带的错误码 |

```javascript
dr1({"result":1,"msg":"Portal协议认证成功！"})
dr1({"result":0,"msg":"终端IP已在线！","ret_code":2})
dr1({"result":0,"msg":"Portal协议认证超时！","ret_code":8})
```

以响应正文的 `result` 判定认证结果。`ret_code=2` 表示终端已在线，当前提交的账号和线路仍需独立认证。认证超时后可先查 `chkstatus` 确认终端状态，再决定是否重试。

下表列出当前客户端识别的错误码。解释沿用门户脚本，新版已确认 `2` 和 `8` 的含义。

| `ret_code` | 含义 |
| --- | --- |
| `1` | 账号或密码不正确 |
| `2` | 终端 IP 已在线 |
| `3` | 系统繁忙 |
| `4` | 未知错误 |
| `5` / `6` | `REQ_CHALLENGE` 失败 / 超时 |
| `7` / `8` | 认证失败 / 超时 |
| `9` / `10` | RADIUS 下线失败 / 超时 |
| `11` | 其他错误 |
| `998` | Portal 协议参数不完整 |

## 客户端调用约定

`Client::login()` 先查询 `chkstatus`。设备已在线时返回 `AlreadyOnline`，仅本次认证收到成功结果时返回 `Authenticated`。客户端也接受字符串 `"1"` 和 `"ok"` 作为操作成功值。切换线路前，通过 `chkstatus.result=0` 确认设备离线。[状态查询接口](./eportal-status.md)、[注销接口](./eportal-logout.md)
