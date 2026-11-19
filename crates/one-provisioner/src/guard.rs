//! Guarda de destino (SSRF): o provisionador só conecta em endereços públicos ou, quando o acesso é
//! `tailscale`, em endereços da tailnet (CGNAT 100.64.0.0/10 e ULA fd7a:115c:a1e0::/48, ou *.ts.net).
//! Loopback, link-local (inclui 169.254.169.254), redes privadas RFC 1918 e ULA fora da tailnet são sempre recusados.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessType {
    Direct,
    Tailscale,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum GuardError {
    #[error("endereço de destino inválido")]
    InvalidHost,
    #[error("destino não permitido: {0}")]
    Denied(&'static str),
}

/// Valida um IP de destino para o tipo de acesso.
pub fn check_ip(ip: IpAddr, access: AccessType) -> Result<(), GuardError> {
    match ip {
        IpAddr::V4(v4) => check_v4(v4, access),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => check_v4(v4, access),
            None => check_v6(v6, access),
        },
    }
}

fn check_v4(ip: Ipv4Addr, access: AccessType) -> Result<(), GuardError> {
    let o = ip.octets();
    // Loopback só existe como destino no ambiente de teste ponta a ponta (feature `e2e`).
    if ip.is_loopback() && cfg!(feature = "e2e") {
        return Ok(());
    }
    if ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() || ip.is_broadcast() {
        return Err(GuardError::Denied("loopback, não especificado, multicast ou broadcast"));
    }
    if o[0] == 169 && o[1] == 254 {
        return Err(GuardError::Denied("link-local (inclui metadados de nuvem)"));
    }
    // CGNAT 100.64.0.0/10: a faixa de IPs da tailnet.
    if o[0] == 100 && (64..=127).contains(&o[1]) {
        return match access {
            AccessType::Tailscale => Ok(()),
            AccessType::Direct => Err(GuardError::Denied("faixa de tailnet fora do acesso Tailscale")),
        };
    }
    if ip.is_private() {
        return Err(GuardError::Denied("rede privada RFC 1918 (inclui redes Docker)"));
    }
    Ok(())
}

fn check_v6(ip: Ipv6Addr, access: AccessType) -> Result<(), GuardError> {
    let s = ip.segments();
    if ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() {
        return Err(GuardError::Denied("loopback, não especificado ou multicast"));
    }
    if (s[0] & 0xffc0) == 0xfe80 {
        return Err(GuardError::Denied("link-local IPv6"));
    }
    // ULA fd7a:115c:a1e0::/48 é a tailnet.
    if s[0] == 0xfd7a && s[1] == 0x115c && s[2] == 0xa1e0 {
        return match access {
            AccessType::Tailscale => Ok(()),
            AccessType::Direct => Err(GuardError::Denied("faixa de tailnet fora do acesso Tailscale")),
        };
    }
    if (s[0] & 0xfe00) == 0xfc00 {
        return Err(GuardError::Denied("rede privada IPv6 (ULA)"));
    }
    Ok(())
}

/// Valida o host informado pelo administrador antes de qualquer conexão.
/// Nomes são aceitos como texto; os endereços resolvidos devem passar por `check_ip` (ver `resolve_checked`).
pub fn check_host(host: &str, access: AccessType) -> Result<(), GuardError> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return check_ip(ip, access);
    }
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    let valid_label = |l: &str| {
        !l.is_empty() && l.len() <= 63 && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') && !l.starts_with('-') && !l.ends_with('-')
    };
    if h.is_empty() || h.len() > 253 || !h.split('.').all(valid_label) {
        return Err(GuardError::InvalidHost);
    }
    if h == "localhost" || h.ends_with(".localhost") || h.ends_with(".local") || h.ends_with(".internal") {
        return Err(GuardError::Denied("nome local ou interno"));
    }
    if h.ends_with(".ts.net") && access != AccessType::Tailscale {
        return Err(GuardError::Denied("nome da tailnet exige acesso Tailscale"));
    }
    Ok(())
}

/// Resolve o host e recusa a conexão se qualquer endereço resolvido for proibido.
/// Retorna os endereços já validados: a conexão usa só esses IPs (evita DNS rebinding).
pub async fn resolve_checked(host: &str, port: u16, access: AccessType) -> Result<Vec<std::net::SocketAddr>, GuardError> {
    check_host(host, access)?;
    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| GuardError::InvalidHost)?
        .collect();
    if addrs.is_empty() {
        return Err(GuardError::InvalidHost);
    }
    for a in &addrs {
        check_ip(a.ip(), access)?;
    }
    Ok(addrs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn public_addresses_are_allowed_for_direct_access() {
        assert!(check_ip(ip("179.199.147.93"), AccessType::Direct).is_ok());
        assert!(check_ip(ip("8.8.8.8"), AccessType::Direct).is_ok());
    }

    #[test]
    fn loopback_link_local_metadata_and_private_are_always_refused() {
        for s in ["127.0.0.1", "0.0.0.0", "169.254.169.254", "10.0.0.5", "172.17.0.1", "192.168.1.6", "224.0.0.1"] {
            assert!(check_ip(ip(s), AccessType::Direct).is_err(), "direct {s}");
            assert!(check_ip(ip(s), AccessType::Tailscale).is_err(), "tailscale {s}");
        }
        assert!(check_ip(ip("::1"), AccessType::Tailscale).is_err());
        assert!(check_ip(ip("fe80::1"), AccessType::Tailscale).is_err());
        assert!(check_ip(ip("::ffff:127.0.0.1"), AccessType::Direct).is_err());
    }

    #[test]
    fn tailnet_addresses_only_with_tailscale_access() {
        assert!(check_ip(ip("100.83.130.39"), AccessType::Tailscale).is_ok());
        assert!(check_ip(ip("100.83.130.39"), AccessType::Direct).is_err());
        assert!(check_ip(ip("fd7a:115c:a1e0::7201:82d2"), AccessType::Tailscale).is_ok());
        assert!(check_ip(ip("fd7a:115c:a1e0::7201:82d2"), AccessType::Direct).is_err());
        // 100.128.0.1 está fora do CGNAT (100.64/10): é endereço público comum.
        assert!(check_ip(ip("100.128.0.1"), AccessType::Direct).is_ok());
    }

    #[test]
    fn hostnames_are_validated_before_resolution() {
        assert!(check_host("flowsharp-servidor.taile8e329.ts.net", AccessType::Tailscale).is_ok());
        assert!(check_host("flowsharp-servidor.taile8e329.ts.net", AccessType::Direct).is_err());
        assert!(check_host("servidor.exemplo.com.br", AccessType::Direct).is_ok());
        assert!(check_host("localhost", AccessType::Direct).is_err());
        assert!(check_host("api.internal", AccessType::Direct).is_err());
        assert!(check_host("bad_host!", AccessType::Direct).is_err());
        assert!(check_host("", AccessType::Direct).is_err());
        assert!(check_host("-x.com", AccessType::Direct).is_err());
    }
}
