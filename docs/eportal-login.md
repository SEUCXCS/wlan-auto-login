# ePortal 校园网登录接口

| 接口 | 方法 | 地址 |
| --- | --- | --- |
| 页面配置 | GET | `http://172.18.254.6:803/eportal/portal/page/loadConfig` |
| 账号登录 | GET | `http://172.18.254.6:803/eportal/portal/login` |

两项接口均使用 URL 查询参数并返回 JSONP。浏览器请求还会附带 `PHPSESSID` Cookie 与门户首页 Referer，服务端是否强制校验待确认。

客户端固定发送 `callback=dr1`。新版门户可能将回调名补零返回为 `dr0001`，客户端同时兼容 `dr1` 与 `dr0001`。

## 页面配置（`loadConfig`）

登录前先通过[状态查询接口](./eportal-status.md)获取本机 IPv4，再调用此接口获取门户的 `program_index`、`page_index` 及认证配置。后续登录、注销与 `online_list` 请求均依赖这两项索引。

### 请求参数

| 参数 | 说明 |
| --- | --- |
| `program_index`、`page_index` | 首次加载传空 |
| `wlan_vlan_id` | VLAN 标识，取自状态查询 |
| `wlan_user_ip`、`wlan_user_ipv6`、`wlan_ac_ip` | 先经标准 Base64 编码，再做 URL 编码；缺失的 IPv6 与 AC IP 传空 |
| `wlan_user_ssid`、`wlan_user_areaid` | SSID 与区域标识，缺失传空 |
| `wlan_ap_mac`、`gw_id` | AP MAC 与网关标识，缺失传 `000000000000` |
| `callback` | JSONP 回调名，客户端使用 `dr1` |
| `jsVersion` | 页面配置使用 `4.X`（登录接口使用 `4.5.1`） |
| `v`、`lang` | 防缓存随机数、`zh` |

```http
GET http://172.18.254.6:803/eportal/portal/page/loadConfig?callback=dr1&program_index=&page_index=&wlan_vlan_id=0&wlan_user_ip=MTAuMC4wLjEwMA%3D%3D&wlan_user_ipv6=&wlan_user_ssid=&wlan_user_areaid=&wlan_ac_ip=&wlan_ap_mac=000000000000&gw_id=000000000000&jsVersion=4.X&v=1234&lang=zh
```

### 响应字段

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `code` | 整数 | `1` 表示成功，`0` 表示失败 |
| `msg` | 字符串 | 门户提示信息 |
| `data.program_index`、`data.page_index` | 字符串 | 方案索引、页面索引 |
| `data.login_method` | 整数 | 认证方式，本校 Portal 账号认证为 `1` |
| `data.account_prefix` | 字符串 | 账号前缀开关，本校启用 `,0,` 前缀 |
| `data.check_online_method` | 字符串 | 在线状态查询方式，为 `1` 时浏览器调用 `online_list` |
| `data.ac_logout`、`data.register_mode` | 字符串 | 注销方式与后台类型，本校均为 `1` |
| `data.enable_r3` | 整数 | PPPoE 代拨开关，缺失时默认 `0` |
| `data.rcn` | 字符串 | 页面随机数，缺失时默认空字符串 |

响应示例（仅列出认证相关字段）：

```javascript
dr0001({
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

`Client::login()` 每次提交认证前都会重新加载页面配置；配置请求失败、返回 `code=0` 或缺少索引时直接终止登录。

## 账号登录（`login`）

### 请求参数

| 参数 | 说明 |
| --- | --- |
| `login_method`、`business_type` | `1`、`1`（Portal 账号认证） |
| `operate`、`is_base64encode` | `portal_login`、`1` |
| `user_account` | 按下文格式构造后，对 UTF-8 字节做标准 Base64 编码 |
| `user_password` | 对密码 UTF-8 字节做标准 Base64 编码 |
| `wlan_user_ip`、`wlan_user_ipv6` | 终端地址明文，IPv6 缺失传空 |
| `wlan_user_mac`、`wlan_vlan_id` | MAC 去除分隔符（缺失传 `000000000000`）；VLAN 取自状态查询 |
| `wlan_ac_ip`、`wlan_ac_name` | 接入控制器 IP 与名称，缺失传空 |
| `program_index`、`page_index` | `loadConfig` 返回的索引 |
| `terminal_type`、`mac_type` | 桌面端固定 `1`、`0` |
| `user_agent` | 浏览器传自身 User-Agent，客户端传 `wlanlogin/<版本>` |
| `enable_r3`、`rcn` | 取自页面配置 |
| `authex_enable`、`uuid` | 传空 |
| `callback`、`jsVersion`、`v` | JSONP 回调名、`4.5.1`、防缓存随机数 |
| `lang` | 与浏览器保持一致，依次传递两个同名参数 `zh-cn` 与 `zh` |

全部查询参数（包括 Base64 填充符 `=`）均需 URL 编码：

```http
GET http://172.18.254.6:803/eportal/portal/login?callback=dr1&login_method=1&is_base64encode=1&user_account=LDAsMjAyNjAwMDAwMEBkaWFueGlu&user_password=ZXhhbXBsZS1wYXNzd29yZA%3D%3D&wlan_user_ip=10.0.0.100&wlan_user_ipv6=&wlan_user_mac=000000000000&wlan_vlan_id=0&wlan_ac_ip=&wlan_ac_name=&authex_enable=&jsVersion=4.5.1&uuid=&terminal_type=1&lang=zh-cn&user_agent=wlanlogin%2F0.1.1&enable_r3=0&mac_type=0&rcn=&operate=portal_login&business_type=1&program_index=example-program&page_index=example-page&v=1234&lang=zh
```

### 账号格式与线路

默认拼接格式为 `,0,<学号><线路后缀>`；仅当页面配置明确返回 `account_prefix=0` 时省略 `,0,` 前缀。

| 线路 | 后缀 |
| --- | --- |
| 临时免费 1（`free1`） | `@dianxin` |
| 免费 2（`free2`） | `@free` |
| 内网（`campus`） | `@xyw` |

> [!NOTE]
> Base64 可直接还原原文，分享日志或抓包样本前需脱敏 `user_account`、`user_password` 与会话 Cookie。

### 响应字段

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `result` | 整数 | `1` 表示成功（客户端兼容 `"1"` 与 `"ok"`），`0` 表示失败 |
| `msg` | 字符串 | 门户提示信息 |
| `ret_code` | 整数 | 失败时附带的错误码 |

```javascript
dr1({"result":1,"msg":"Portal协议认证成功！"})
dr1({"result":0,"msg":"终端IP已在线！","ret_code":2})
dr1({"result":0,"msg":"Portal协议认证超时！","ret_code":8})
```

认证超时后应先查询 `chkstatus` 确认终端状态，再决定是否重试。错误码定义如下（来源于门户前端脚本，新版已实测确认 `2` 与 `8`）：

| `ret_code` | 含义 |
| --- | --- |
| `1` | 账号或密码错误 |
| `2` | 终端 IP 已在线 |
| `3` | 系统繁忙 |
| `4` | 未知错误 |
| `5` / `6` | `REQ_CHALLENGE` 失败 / 超时 |
| `7` / `8` | 认证失败 / 超时 |
| `9` / `10` | RADIUS 下线失败 / 超时 |
| `11` | 其他错误 |
| `998` | Portal 协议参数不完整 |

## 客户端流程

`Client::login()` 先查询 `chkstatus`。终端已在线时返回 `AlreadyOnline`，状态未知时停止，确认离线后才加载配置并提交认证。CLI 和守护进程通过 `prepare_login()` 完成门户预检，在实际提交前复核当前 Wi-Fi 会话。切换线路前先调用[注销接口](./eportal-logout.md)，待 `chkstatus` 确认离线后再发起登录。
