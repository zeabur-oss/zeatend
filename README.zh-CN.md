<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/cover-zh-CN-dark.avif">
  <img alt="Zeatend：一台 Linux 服务器，变成可以直接用的集群。zeatend apply 把 Linux 服务器变成运行中的 Kubernetes 集群。" src="docs/images/cover-zh-CN-light.avif">
</picture>

<!-- The names of the languages, each in its own script. -->
<!-- vale ZeaburZhCN.Script = NO -->
[English](README.md) | [繁體中文](README.zh-TW.md) | 简体中文
<!-- vale ZeaburZhCN.Script = YES -->

# Zeabur Zeatend

Zeatend 是管理 Kubernetes 集群的工具，思路和 [k3sup](https://github.com/alexellis/k3sup) 相近。它通过 SSH 在主机上安装和管理 K3s、RKE2 和 KubeSolo 集群，并提供终端界面。

Zeatend 由 Zeabur 项目开发，目前还处在早期开发阶段。

## 使用方法

目前 `zeatend` 需要以 root 身份，在它要安装的那台 Linux 主机上运行。不带命令运行 `sudo zeatend`，会打开安装界面，可以选择英文、繁体中文或简体中文：

<img alt="简体中文的服务器界面：主机信息、K3s、RKE2 或 KubeSolo 的选项，以及主机检查。" src="docs/screens/80x24/zh_CN-server.svg" width="744">

界面会显示主机信息和检查结果，让你选择发行版，填写 API 服务器的地址、DNS 名称和节点名称，最后显示安装计划。你的选择会写入 `/etc/zeatend/config.yaml`，安装过程和 `zeatend apply` 一样。所有界面在 80x24 和 120x36 下的样子都在 [docs/screens](docs/screens)。语言和主机检查一样取自区域设置，按 F2 可以切换。Linux 控制台这类不能显示中文的终端，会改用英文和 ASCII 边框。

不打开界面时，`zeatend` 读取一个配置文件（默认是 `/etc/zeatend/config.yaml`），安装按 CIS 基准加固的 K3s、RKE2 或 KubeSolo：

```yaml
distribution: k3s # k3s（默认）、rke2 或 kubesolo
host:
  tlsSan: [k8s.example.com]
```

K3s（`v1.37.2+k3s1`）和 RKE2（`v1.37.2+rke2r1`）安装 Kubernetes 1.37。KubeSolo 还没有 1.37 版，所以 KubeSolo `v1.3.0` 安装的是 Kubernetes 1.36.6。

K3s 和 RKE2 按上游的默认设置，部署自带的组件，例如 Traefik。要改用自己安装的版本，就用 `disable` 关闭，例如 `disable: [traefik, metrics-server]`。`local-storage` 会关闭 K3s 的 local-path 存储类。KubeSolo 没有这些组件。由 zeabur-installer 安装的集群，`disable` 一开始就是 installer 关闭的那些组件。

`apiServer.authenticationConfig` 是 API 服务器的[认证配置](https://kubernetes.io/docs/reference/access-authn-authz/authentication/#using-authentication-configuration)，按 Kubernetes 文档的格式写，例如让 API 服务器接受某个 OpenID Connect 签发者的令牌：

```yaml
apiServer:
  authenticationConfig:
    apiVersion: apiserver.config.k8s.io/v1
    kind: AuthenticationConfiguration
    jwt:
      - issuer:
          url: https://identity.example.com
          audiences: [kubernetes]
        claimMappings:
          username: {claim: preferred_username, prefix: "zeabur:"}
          groups: {claim: groups, prefix: "zeabur:"}
```

在服务器节点上，zeatend 会把它写成文件交给 API 服务器。字段由 Kubernetes 在 API 服务器启动或重新读取文件时检查。在 K3s 和 RKE2 上，如果配置中没有 `anonymous`，zeatend 会加上 `anonymous: {enabled: false}`，匿名请求就保持关闭。修改配置后，下一次 `apply` 就会生效。

- `zeatend plan` 显示 `apply` 会安装哪些内容，并运行主机检查；加上 `--json` 会以 JSON 输出计划。
- `zeatend apply` 负责安装，或让现有的安装与配置文件一致。重复运行也没有问题。`apply --plan <文件>` 会应用 `plan --json` 写入文件的计划；如果计划要写入的文件、安装的版本或安装记录在那之后发生了变化，就不会做任何更改。
- `zeatend uninstall` 删除 K3s、RKE2 或 KubeSolo，以及 zeatend 写入的文件。它会先列出要删除和保留的内容，并询问是否继续（`--yes` 跳过询问）。除非加上 `--delete-volumes`，存储卷都会保留。

在使用 systemd 的主机上，`apply` 会在临时单元 `zeatend-install-<id>.service` 中运行安装，并显示进度。SSH 连接断开后，安装会继续。再运行一次 `sudo zeatend apply` 就能接着查看：安装单元运行期间，`apply` 会从 journal 显示该单元的进度，不会另外开始新的安装。完整输出可以用 `journalctl -u 'zeatend-install-*'` 查看。`apply --foreground` 不创建单元，直接在当前的 zeatend 进程中运行安装，`apply --json` 则以 JSON 逐行输出进度。

配置文件中有 `join` 部分时，这台主机会加入现有的 K3s 或 RKE2 集群：

```yaml
distribution: k3s
join:
  server: https://10.0.0.5:6443 # RKE2 用 9345 端口
  role: worker # RKE2 也可以设为 server
  tokenFile: /etc/zeatend/join-token # 权限 0600
```

## 参与贡献

请参阅 [CONTRIBUTING.md](CONTRIBUTING.md)。报告安全问题的方式见 [SECURITY.md](SECURITY.md)。

## 许可证

[Apache-2.0](LICENSE)。
