mod at_parser {
    use st67w611::at::{
        parser::parse_csv, parser::parse_ip, parser::parse_line, parser::unquote, AtResponse,
    };

    #[test]
    fn test_parse_line_ok() {
        let result = parse_line("OK").unwrap();
        assert_eq!(result, Some(AtResponse::Ok));
    }

    #[test]
    fn test_parse_line_error() {
        let result = parse_line("ERROR").unwrap();
        assert_eq!(result, Some(AtResponse::Error));
    }

    #[test]
    fn test_parse_csv() {
        let result = parse_csv("1,\"test\",3");
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].as_str(), "1");
        assert_eq!(result[1].as_str(), "test");
        assert_eq!(result[2].as_str(), "3");
    }

    #[test]
    fn test_unquote() {
        assert_eq!(unquote("\"test\""), "test");
        assert_eq!(unquote("test"), "test");
        assert_eq!(unquote("\""), "\"");
    }

    #[test]
    fn test_parse_ip() {
        let ip = parse_ip("192.168.1.100").unwrap();
        assert_eq!(ip.octets(), [192, 168, 1, 100]);
    }
}
