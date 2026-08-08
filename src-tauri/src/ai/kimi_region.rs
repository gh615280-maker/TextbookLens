use crate::domain::KimiApiRegion;

pub const KIMI_CN_ORIGIN: &str = "https://api.moonshot.cn/v1/";
pub const KIMI_INTERNATIONAL_ORIGIN: &str = "https://api.moonshot.ai/v1/";

pub const fn production_origin(region: KimiApiRegion) -> &'static str {
    match region {
        KimiApiRegion::Cn => KIMI_CN_ORIGIN,
        KimiApiRegion::International => KIMI_INTERNATIONAL_ORIGIN,
    }
}
