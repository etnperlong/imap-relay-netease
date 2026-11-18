use std::{env, io};

pub fn from_env() -> io::Result<&'static str> {
    match env::var("IMAP_ID_PROFILE").as_deref() {
        Err(env::VarError::NotPresent) | Ok("apple-mail") => {
            Ok(r#"ID ("name" "Apple Mail" "vendor" "Apple Inc.")"#)
        }
        Ok("thunderbird") => Ok(r#"ID ("name" "Thunderbird" "vendor" "Mozilla")"#),
        Ok("outlook") => Ok(r#"ID ("name" "Microsoft Outlook" "vendor" "Microsoft")"#),
        Ok("foxmail") => Ok(r#"ID ("name" "Foxmail" "vendor" "Tencent")"#),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "IMAP_ID_PROFILE must be apple-mail, thunderbird, outlook, or foxmail",
        )),
    }
}
