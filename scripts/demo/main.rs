mod retry;

use retry::retry_delay;

fn main() {
    let endpoint = "https://example.invalid/jobs";

    for attempt in 0..4 {
        let delay = retry_delay(attempt);
        println!("Attempt {attempt}: retry {endpoint} after {delay:?}");
    }
}
