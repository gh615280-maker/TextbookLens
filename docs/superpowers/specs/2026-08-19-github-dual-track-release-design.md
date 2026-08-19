# TextbookLens GitHub 双轨发布设计

## 目标

将 TextbookLens 作为公开的 Apache-2.0 项目发布到 `gh615280-maker/TextbookLens`，同时服务两类用户：普通用户可以从 GitHub Releases 下载 Windows 11 x64 安装包直接安装；开发者可以克隆完整源码，自行构建、测试和修改。

## 发布结构

公开仓库保留完整源码、锁定依赖、测试、构建脚本、贡献指南、安全政策、许可与第三方声明。仓库首页把“普通用户下载”和“开发者构建”分成两个明确入口，避免普通用户误下载源码压缩包。

`v0.1.0` GitHub Release 提供以下资产：

- `TextbookLens_0.1.0_x64-setup.exe`：普通用户首选安装包。
- `TextbookLens_0.1.0_x64_en-US.msi`：面向系统管理员和需要 MSI 的用户。
- `TextbookLens_User_Guide.docx`：离线用户指南。
- `SHA256SUMS.txt`：所有上传资产的 SHA-256 校验值。

GitHub 自动附带的 Source code ZIP/TAR.GZ 作为源码快照；开发者仍以克隆仓库为推荐方式。

## 版本与自动化

现有 `v0.1.0` 标签及安装包保持首版版本号。新增 GitHub Actions 发布工作流：由语义化版本标签触发，在 Windows runner 上使用锁定的 Node、npm、Rust 和 Tauri 版本构建安装包，运行发布相关检查，生成校验文件并上传到对应 GitHub Release。工作流仅申请发布所需的最小 `contents: write` 权限。

首版发布可以使用已通过本地发布检查的现有安装包，但上传前必须重新核对文件哈希、构建来源和敏感文件扫描结果。后续版本以自动化工作流生成的资产为准。

## 签名与用户提示

当前未提供 Windows 代码签名证书，因此 `v0.1.0` 按未签名测试版发布。Release 说明和下载说明必须明确：Windows SmartScreen 可能显示未知发布者警告。发布流程预留签名步骤，但在配置证书前不伪装为已签名或正式可信发布。

## 安全与仓库边界

上传前运行现有敏感文件检查，并检查 Git 历史、未跟踪文件和大文件。`.env`、API 密钥、本地数据库、构建缓存、测试报告和开发机路径不得进入仓库或 Release。安装包、指南与校验文件只作为 Release 资产上传，不把生成的安装包提交进 Git 历史。

## 验证与验收

发布完成需满足：

1. 公开仓库可匿名访问并能克隆。
2. README 同时提供普通用户下载入口和开发者构建入口。
3. `v0.1.0` Release 可匿名访问，EXE、MSI、用户指南和校验文件均可下载。
4. 上传文件的 SHA-256 与 `SHA256SUMS.txt` 一致。
5. 仓库 CI 配置有效，发布工作流可由后续版本标签重复执行。
6. Release 明确标注 Windows 11 x64、未签名状态和 SmartScreen 提示。

## 不在本次范围

本次不购买或配置代码签名证书，不发布 macOS/Linux 版本，不搭建独立下载站，不引入自动更新服务，也不改变应用功能。
