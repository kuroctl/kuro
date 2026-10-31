//! Gabarit des e-mails (ADR-0046) : les couleurs du panel (thème sombre, violet primaire), le
//! logo Kuro en image inline (CID), un tableau de 600 px et des styles en ligne, ce que les
//! clients de messagerie savent afficher. Chaque message a aussi sa version texte.

/// Identifiant de l'image inline du logo (`<img src="cid:…">`).
pub const LOGO_CID: &str = "kuro-logo";
/// Logo Kuro (mot-symbole clair, 360 × 85), tiré de `site/public/assets/dark.png`.
pub const LOGO_PNG: &[u8] = include_bytes!("assets/logo.png");

// Couleurs du panel en sombre (`panel/src/assets/main.css`), en hexadécimal pour les clients.
const PAGE: &str = "#10121a";
const CARD: &str = "#1c1f2b";
const BORDER: &str = "#2e3242";
const TEXT: &str = "#eef0f5";
const MUTED: &str = "#a3a7b7";
const PRIMARY: &str = "#7c6cf2";

/// Contenu d'un message : un titre, des paragraphes, un bouton facultatif, une note en pied.
#[derive(Debug, Clone, Default)]
pub struct Email {
    /// Nom du projet, en pied et dans le sujet.
    pub brand: String,
    pub title: String,
    pub paragraphs: Vec<String>,
    /// Bouton : libellé et URL.
    pub action: Option<(String, String)>,
    /// Petite ligne grise sous le contenu (validité d'un lien, origine du message…).
    pub note: Option<String>,
}

impl Email {
    /// Sujet « [Projet] Titre ».
    pub fn subject(&self) -> String {
        format!("[{}] {}", self.brand, self.title)
    }

    /// Version texte : titre, paragraphes, le lien sur sa propre ligne, la note, la signature.
    pub fn text(&self) -> String {
        let mut text = format!("{}\n\n", self.title);
        for paragraph in &self.paragraphs {
            text.push_str(paragraph);
            text.push_str("\n\n");
        }
        if let Some((label, url)) = &self.action {
            text.push_str(&format!("{label} :\n{url}\n\n"));
        }
        if let Some(note) = &self.note {
            text.push_str(&format!("{note}\n\n"));
        }
        text.push_str(&format!("-- \n{}\n", self.brand));
        text
    }

    /// Version HTML, logo compris (image inline `cid:kuro-logo`).
    pub fn html(&self) -> String {
        let mut body = String::new();
        for paragraph in &self.paragraphs {
            body.push_str(&format!(
                "<p style=\"margin:0 0 14px;font-size:15px;line-height:1.6;color:{TEXT}\">{}</p>",
                escape(paragraph)
            ));
        }
        if let Some((label, url)) = &self.action {
            body.push_str(&format!(
                "<table role=\"presentation\" cellspacing=\"0\" cellpadding=\"0\" border=\"0\" style=\"margin:24px 0 10px\"><tr>\
                 <td style=\"border-radius:10px;background:{PRIMARY}\">\
                 <a href=\"{url}\" style=\"display:inline-block;padding:12px 22px;font-size:15px;font-weight:600;color:#ffffff;text-decoration:none;border-radius:10px\">{label}</a>\
                 </td></tr></table>\
                 <p style=\"margin:0 0 14px;font-size:12px;line-height:1.5;color:{MUTED};word-break:break-all\">{url}</p>",
                url = escape(url),
                label = escape(label),
            ));
        }
        if let Some(note) = &self.note {
            body.push_str(&format!(
                "<p style=\"margin:18px 0 0;font-size:12px;line-height:1.5;color:{MUTED}\">{}</p>",
                escape(note)
            ));
        }
        format!(
            "<!doctype html><html lang=\"fr\"><head><meta charset=\"utf-8\">\
             <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
             <meta name=\"color-scheme\" content=\"dark\"><title>{title}</title></head>\
             <body style=\"margin:0;padding:0;background:{PAGE};font-family:-apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif\">\
             <table role=\"presentation\" width=\"100%\" cellspacing=\"0\" cellpadding=\"0\" border=\"0\" style=\"background:{PAGE}\"><tr><td align=\"center\" style=\"padding:32px 16px\">\
             <table role=\"presentation\" width=\"600\" cellspacing=\"0\" cellpadding=\"0\" border=\"0\" style=\"max-width:600px;width:100%;background:{CARD};border:1px solid {BORDER};border-radius:16px\">\
             <tr><td align=\"center\" style=\"padding:28px 32px 20px\">\
             <img src=\"cid:{LOGO_CID}\" width=\"150\" height=\"35\" alt=\"Kuro\" style=\"display:block;width:150px;height:auto;border:0\">\
             </td></tr>\
             <tr><td style=\"padding:0 32px\"><div style=\"height:1px;background:{BORDER}\"></div></td></tr>\
             <tr><td style=\"padding:28px 32px 8px\">\
             <h1 style=\"margin:0 0 18px;font-size:24px;line-height:1.3;font-weight:700;color:{TEXT}\">{title}</h1>\
             {body}\
             </td></tr>\
             <tr><td style=\"padding:20px 32px 28px\">\
             <div style=\"height:1px;background:{BORDER};margin-bottom:16px\"></div>\
             <p style=\"margin:0;font-size:12px;line-height:1.5;color:{MUTED}\">{brand} · message automatique, ne pas répondre · automatic message, do not reply</p>\
             </td></tr>\
             </table></td></tr></table></body></html>",
            title = escape(&self.title),
            brand = escape(&self.brand),
        )
    }
}

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_template_escapes_and_carries_the_logo_and_the_action() {
        let email = Email {
            brand: "Studio <Nuage>".to_owned(),
            title: "Invitation".to_owned(),
            paragraphs: vec!["Bonjour & bienvenue".to_owned()],
            action: Some((
                "Ouvrir".to_owned(),
                "https://panel.example.com/x?a=1&b=2".to_owned(),
            )),
            note: Some("Valable 48 h".to_owned()),
        };
        let html = email.html();
        assert!(html.contains("cid:kuro-logo"));
        assert!(html.contains("Studio &lt;Nuage&gt;"));
        assert!(html.contains("Bonjour &amp; bienvenue"));
        assert!(html.contains("href=\"https://panel.example.com/x?a=1&amp;b=2\""));
        assert!(!html.contains("<Nuage>"));
        let text = email.text();
        assert!(text.contains("Ouvrir :\nhttps://panel.example.com/x?a=1&b=2\n"));
        assert_eq!(email.subject(), "[Studio <Nuage>] Invitation");
        assert!(!LOGO_PNG.is_empty());
    }
}
