use rand::{self, distributions::Slice, Rng};

pub fn rand_hex(len: usize) -> String {
    const HEX: [char; 16] = [
        '0', '1', '2', '3',
        '4', '5', '6', '7',
        '8', '9', 'a', 'b',
        'c', 'd', 'e', 'f'
    ];

    return rand::thread_rng()
        .sample_iter(Slice::new(&HEX).unwrap())
        .take(len).collect()
}