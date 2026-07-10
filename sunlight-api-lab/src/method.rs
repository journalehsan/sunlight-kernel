#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
}

impl HttpMethod {
    pub const ALL: [Self; 7] = [
        Self::Get,
        Self::Post,
        Self::Put,
        Self::Patch,
        Self::Delete,
        Self::Head,
        Self::Options,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
            Self::Head => "HEAD",
            Self::Options => "OPTIONS",
        }
    }

    pub const fn allows_body(self) -> bool {
        matches!(self, Self::Post | Self::Put | Self::Patch | Self::Delete)
    }

    pub const fn index(self) -> usize {
        match self {
            Self::Get => 0,
            Self::Post => 1,
            Self::Put => 2,
            Self::Patch => 3,
            Self::Delete => 4,
            Self::Head => 5,
            Self::Options => 6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyFormat {
    RawText,
    Json,
    Xml,
    FormUrlEncoded,
}

impl BodyFormat {
    pub const ALL: [Self; 4] = [Self::RawText, Self::Json, Self::Xml, Self::FormUrlEncoded];

    pub const fn label(self) -> &'static str {
        match self {
            Self::RawText => "Raw",
            Self::Json => "JSON",
            Self::Xml => "XML",
            Self::FormUrlEncoded => "Form",
        }
    }

    pub const fn index(self) -> usize {
        match self {
            Self::RawText => 0,
            Self::Json => 1,
            Self::Xml => 2,
            Self::FormUrlEncoded => 3,
        }
    }

    pub const fn default_content_type(self) -> &'static str {
        match self {
            Self::RawText => "text/plain",
            Self::Json => "application/json",
            Self::Xml => "application/xml",
            Self::FormUrlEncoded => "application/x-www-form-urlencoded",
        }
    }
}
