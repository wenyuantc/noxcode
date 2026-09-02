use std::io;

#[derive(Debug, thiserror::Error)]
pub(crate) enum SshError {
    #[error("SSH 协议错误: {0}")]
    Russh(#[from] russh::Error),
    #[error("SSH 密钥错误: {0}")]
    Keys(#[from] russh::keys::Error),
    #[error("IO 错误: {0}")]
    Io(#[from] io::Error),
    #[error("连接超时")]
    ConnectTimeout,
    #[error("认证失败，服务器仍接受: {remaining_methods}")]
    AuthFailed { remaining_methods: String },
    #[error("无法加载私钥: {0}")]
    KeyLoad(String),
    #[error("未知主机 {host}:{port}（指纹 {fingerprint}），当前策略拒绝自动信任")]
    HostKeyUnknownRejected {
        host: String,
        port: u16,
        fingerprint: String,
    },
    #[error("主机密钥已变更（known_hosts 第 {line} 行，文件 {path}），已拒绝连接以防止中间人攻击")]
    HostKeyChanged { line: usize, path: String },
    #[error("暂不支持主机证书认证")]
    HostCertificateUnsupported,
    #[error("主机指纹确认超时")]
    TrustPromptTimeout,
    #[error("用户拒绝了该主机指纹")]
    TrustPromptRejected,
    #[error("远端通道请求失败")]
    ChannelFailure,
    #[error("远端命令执行超时")]
    CommandTimeout,
    #[error("SSH 连接已断开")]
    ConnectionLost,
    #[error("{0}")]
    Config(String),
}

impl From<SshError> for String {
    fn from(value: SshError) -> Self {
        value.to_string()
    }
}
