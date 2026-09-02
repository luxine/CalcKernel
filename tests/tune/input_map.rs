use calckernel::{TuneInputMapEntry, decode_input_map, encode_input_map};

#[test]
fn input_map_schema_one_round_trips_and_rejects_trailing_bytes() {
    let entries = vec![TuneInputMapEntry {
        logical_path: "data/search.bin".to_owned(),
        staged_basename:
            "00000000-4242424242424242424242424242424242424242424242424242424242424242.bin"
                .to_owned(),
        bytes: 17,
        digest: [0x42; 32],
    }];

    let encoded = encode_input_map(&entries).expect("encode");
    assert!(encoded.starts_with(b"CKTIMAP1\0\0\0\x01"));
    assert_eq!(decode_input_map(&encoded).expect("decode"), entries);

    let mut trailing = encoded;
    trailing.push(0);
    assert!(decode_input_map(&trailing).is_err());
}
