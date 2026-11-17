use fossa_jupiter::FossaAmm;
use jupiter_amm_test_kit::PoolTest;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let rpc = std::env::var("RPC")?;
    let pool = args.next().ok_or("Expected a pool address")?.parse()?;
    if args.next().is_some() {
        return Err("Expected one pool address".into());
    }
    PoolTest::new(pool).regenerate_fixtures::<FossaAmm>(&rpc)?;
    Ok(())
}
