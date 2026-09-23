# ePortal 校园网注销协议

本文记录当前门户注销页实际使用的请求结构，作为自动登录实现中“注销”能力的协议基础。

> 本次只查看了 `http://172.18.254.6/` 的页面结构、按钮逻辑和公开静态脚本，没有点击注销，也没有实际调用注销接口。以下结论尚未经过真实注销响应验证。

## 1. 当前页面如何触发注销

访问：

```http
GET http://172.18.254.6/
```

当前会话已登录时，页面标题为“注销页”，页面包含一个“注销”按钮：

```html
<input name="logout" type="button" onclick="javascript:wc()" value="注 销">
```

页面脚本中的调用链为：

```text
wc()
  -> 弹出注销确认框
  -> exit()
  -> logout.init()
  -> logout.logout_portal()
  -> logout.portal_logout(data)
  -> JSONP 请求注销接口
```

页面脚本会先检查 `wlan_user_ip`。如果 IP 不存在或仍为 `000.000.000.000`，脚本会中止注销，因此注销前必须先获得当前设备的校园网 IP。

## 2. 主注销接口

当前页面分支使用的接口是：

```http
GET http://172.18.254.6:801/eportal/?c=Portal&a=logout
```

完整请求结构：

```text
http://172.18.254.6:801/eportal/?c=Portal&a=logout&callback=<JSONP回调名>&login_method=1&user_account=drcom&user_password=123&ac_logout=0&register_mode=1&wlan_user_ip=<当前设备校园网IP>&wlan_user_ipv6=<当前设备IPv6或空>&wlan_vlan_id=<VLAN ID>&wlan_user_mac=<设备MAC或占位值>&wlan_ac_ip=<接入控制器IP或空>&wlan_ac_name=<接入控制器名称或空>&jsVersion=3.3.3&v=<随机数>
```

### 注销参数

| 参数 | 页面中的值或来源 | 是否必需 | 说明 |
| --- | --- | --- | --- |
| `c` | `Portal` | 是 | 固定值。 |
| `a` | `logout` | 是 | 固定值，表示标准 Portal 注销。 |
| `callback` | 动态生成 | 是 | JSONP 回调名。页面脚本从 `1000` 递增生成，例如 `dr1001`、`dr1002`。 |
| `login_method` | `page.loginMethod` | 是 | 当前加载的 IP 匹配配置中为 `1`，对应 PORTAL 协议。 |
| `user_account` | `drcom` | 是 | 注销接口使用的固定占位账号，不是用户的学号。 |
| `user_password` | `123` | 是 | 注销接口使用的固定占位密码，不是用户的校园网密码。 |
| `ac_logout` | `0` | 是 | 当前页面配置值为 `0`，表示不走额外的 AC 注销接口。 |
| `register_mode` | `1` | 是 | 当前页面配置值。 |
| `wlan_user_ip` | `term.ip` | 是 | 当前设备的校园网 IP；为空时页面不会发起注销。 |
| `wlan_user_ipv6` | `term.ipv6` | 是 | 默认为空；只有页面通过 `UserV6IP` 参数得到 IPv6 地址时才赋值。 |
| `wlan_vlan_id` | `term.vlan` | 是 | 当前设备 VLAN ID；页面无法读取时默认可能为 `1`。 |
| `wlan_user_mac` | `term.mac` | 是 | 当前设备 MAC；页面无法读取时可能使用 `000000000000`。 |
| `wlan_ac_ip` | `term.wlanacip` | 是 | 页面从 URL 参数中读取。当前登录协议已知的接入控制器 IP 为 `172.18.255.6`。 |
| `wlan_ac_name` | `term.wlanacname` | 是 | 页面从 URL 参数中读取。当前登录协议已知的接入控制器名称为 `NAS`。 |
| `jsVersion` | `3.3.3` | 是 | 当前页面脚本版本。 |
| `v` | 随机数 | 是 | 页面每次生成 `500` 至 `10499` 之间的随机值，用于避免缓存。 |

`user_account=drcom` 和 `user_password=123` 是注销脚本内部写死的协议占位值，不能与登录时使用的真实学号和密码混淆。

### 请求示例

以下示例只展示拼接方式，所有尖括号内容都需要替换：

```text
http://172.18.254.6:801/eportal/?c=Portal&a=logout&callback=dr1001&login_method=1&user_account=drcom&user_password=123&ac_logout=0&register_mode=1&wlan_user_ip=10.0.0.100&wlan_user_ipv6=&wlan_vlan_id=1&wlan_user_mac=001122334455&wlan_ac_ip=172.18.255.6&wlan_ac_name=NAS&jsVersion=3.3.3&v=1234
```

## 3. 成功响应判断

页面脚本使用 JSONP 接收响应，并认为以下结果表示注销成功：

```text
json.result == 1
json.result == "ok"
```

其他结果会被页面视为注销失败。该判断来自页面脚本，尚未通过真实注销请求验证，也不代表已经掌握完整的错误码列表。

## 4. 兼容或旧式注销路径

页面内联配置还提供了以下 Dr.COM 客户端元数据：

```text
authlogoutport=80
authlogoutpath='/eportal/?c=ACSetting&a=Logout&ver=1.0'
```

但当前“注销”按钮的逻辑没有使用这条路径。按当前脚本分支，实际调用的是：

```text
/eportal/?c=Portal&a=logout
```

脚本还存在一个非 Portal 协议的回退分支：

```text
http://172.18.254.6/drcom/logout
```

该分支仅在 `page.loginMethod == 0` 时使用。当前 Portal 协议配置为 `1`，因此不属于当前页面的主路径。

## 5. 实现注销时的要求

1. 注销必须有明确的用户操作触发，不能在程序启动或网络恢复时自动执行。
2. 发起注销前必须确认 `wlan_user_ip` 有效，不能使用空值或 `000.000.000.000`。
3. 应优先复用登录流程已经取得的 VLAN、MAC、接入控制器 IP 和接入控制器名称。
4. `wlan_ac_ip` 和 `wlan_ac_name` 在注销脚本中来自页面上下文，不保证始终有值；当前登录协议中已知值为 `172.18.255.6` 和 `NAS`。
5. 所有参数都应进行 URL 编码，`callback`、`v` 可以按页面逻辑动态生成。
6. 网页自身会先显示确认框。CLI 按当前产品要求以显式 `logout` 命令作为用户操作意图，不再添加二次确认；同时协调 daemon 暂停当前连接期间的自动登录，防止立即重新登录。
7. 页面脚本无论成功或失败都会修改界面状态，因此真实请求应避免重复并发发送。

CLI 的 `logout --forget` 是本地完全登出选项，不向门户增加额外参数。它会停止所选范围的后台进程，并删除对应的用户密钥库条目或机器凭据。离开 `Student_CX` 时跳过门户注销；门户失败或 Wi-Fi 状态无法确认时仍尝试删除本地凭据，但会明确返回未完成的错误。此选项不会在失败后恢复自动登录，也不会把“凭据已删除”当作远端会话已注销的证明。

## 6. 尚未确认的事项

- 注销接口接受的最小参数集合；
- `wlan_ac_ip`、`wlan_ac_name` 为空时是否仍能成功注销；
- 真实成功和失败响应的完整示例；
- 是否需要 Cookie、会话或特定请求头；
- 注销后当前设备 IP 和页面状态如何变化；
- 多次注销、会话过期和接口超时的处理方式。

在完成受控验证前，实现应把此处内容视为当前门户页面所展示的已知协议，而不是完整的通用 ePortal 注销规范。

## 7. 相关文档

- 设备 IP、接入信息及认证状态：[Dr.COM 校园网状态查询协议](./eportal-status.md)
- [ePortal 校园网登录协议](./eportal-login.md)
