#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesktopRequest {
    Launch {
        game_id: i64,
        variant_id: Option<i64>,
    },
    Search(String),
}

pub fn positive_id(value: &str) -> Result<i64, String> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("Expected a positive numeric game or variant ID".into());
    }
    value
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| "Game or variant ID is out of range".into())
}

pub fn parse_request(
    game: Option<&str>,
    variant: Option<&str>,
    search: Option<&str>,
) -> Result<Option<DesktopRequest>, String> {
    if game.is_some() && search.is_some() || variant.is_some() && game.is_none() {
        return Err("Use --launch-game with an optional --variant, or --search".into());
    }
    if let Some(game) = game {
        return Ok(Some(DesktopRequest::Launch {
            game_id: positive_id(game)?,
            variant_id: variant.map(positive_id).transpose()?,
        }));
    }
    if let Some(search) = search {
        if search.len() > 4096 {
            return Err("Search text is too long".into());
        }
        return Ok(Some(DesktopRequest::Search(search.to_string())));
    }
    Ok(None)
}

#[derive(Default)]
pub struct PendingRequests {
    ready: bool,
    waiting: Vec<DesktopRequest>,
}

impl PendingRequests {
    pub fn submit(&mut self, request: DesktopRequest) -> Result<Vec<DesktopRequest>, String> {
        if self.ready {
            return Ok(vec![request]);
        }
        if !self.waiting.contains(&request) {
            if self.waiting.len() >= 32 {
                return Err("Too many pending desktop requests".into());
            }
            self.waiting.push(request);
        }
        Ok(Vec::new())
    }

    pub fn library_ready(&mut self) -> Vec<DesktopRequest> {
        self.ready = true;
        std::mem::take(&mut self.waiting)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_positive_id_rejects_malformed_and_out_of_range_values() {
        assert_eq!(positive_id("42").unwrap(), 42);
        for value in ["", "0", "-1", "+1", "1; rm", " 2", "9999999999999999999999"] {
            assert!(positive_id(value).is_err(), "{value}");
        }
    }

    #[test]
    fn test_parse_request_launch_and_search_are_exclusive() {
        assert_eq!(
            parse_request(Some("2"), Some("3"), None).unwrap(),
            Some(DesktopRequest::Launch {
                game_id: 2,
                variant_id: Some(3)
            })
        );
        assert_eq!(
            parse_request(None, None, Some("Pokémon")).unwrap(),
            Some(DesktopRequest::Search("Pokémon".into()))
        );
        assert!(parse_request(Some("2"), None, Some("title")).is_err());
        assert!(parse_request(None, Some("3"), None).is_err());
        assert!(parse_request(None, None, Some(&"x".repeat(4097))).is_err());
        assert_eq!(parse_request(None, None, None).unwrap(), None);
    }

    #[test]
    fn test_pending_requests_wait_for_library_and_deduplicate() {
        let mut pending = PendingRequests::default();
        let request = DesktopRequest::Launch {
            game_id: 2,
            variant_id: None,
        };
        assert!(pending.submit(request.clone()).unwrap().is_empty());
        assert!(pending.submit(request.clone()).unwrap().is_empty());
        assert_eq!(pending.library_ready(), vec![request.clone()]);
        assert_eq!(pending.submit(request.clone()).unwrap(), vec![request]);
        assert!(pending.library_ready().is_empty());
    }

    #[test]
    fn test_pending_requests_bound_cold_start_queue() {
        let mut pending = PendingRequests::default();
        for game_id in 1..=32 {
            pending
                .submit(DesktopRequest::Launch {
                    game_id,
                    variant_id: None,
                })
                .unwrap();
        }
        assert!(pending
            .submit(DesktopRequest::Search("extra".into()))
            .is_err());
    }
}
