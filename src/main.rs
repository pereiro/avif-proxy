use tide::{Request, Response};
use clap::{Parser};
use std::sync::{RwLock, Arc};
use anyhow::anyhow;
use simplelog::*;
use imgref::ImgVec;
use ravif::{RGBA8, encode_rgba, ColorSpace};

#[derive(Parser,Clone)]
#[clap(version = "1.0", author = "Wildberries")]
struct Opts {
    #[clap(short, long, default_value = "127.0.0.1:8080")]
    listen: String,
    #[clap(short, long, default_value = "https://images.wbstatic.net")]
    jpeg_backend: String,
    #[clap(short, long, parse(from_occurrences))]
    verbose: i32,
    #[clap(short, long, default_value = "80.0")]
    quality: f32,
    #[clap(short, long, default_value = "6")]
    speed: u8,
}

#[async_std::main]
async fn main() -> tide::Result<()> {
    let opts = Opts::parse();
    let listen_addr = opts.listen.clone();

    TermLogger::init(    match opts.verbose{
        0 => LevelFilter::Warn,
        1 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    },simplelog::Config::default(),TerminalMode::Mixed,ColorChoice::Auto).unwrap();

    let opts = Arc::new(RwLock::new(opts));


    let mut app = tide::with_state(opts);
    app.at("/*").get(handler);
    println!("Starting HTTP server at {}",listen_addr);
    app.listen(listen_addr).await?;
    Ok(())
}

async fn handler(req: Request<Arc<RwLock<Opts>>>) -> tide::Result {
    let opts: Opts;
        {
            opts = req.state().read().unwrap().clone();
        }
    let mut response = surf::client().send(surf::get(opts.jpeg_backend + req.url().path())).await?;
    if !response.status().is_success(){
        return Err(tide::Error::new(response.status(),anyhow!("{}",response.status().to_string())))
    }
    let payload = response.body_bytes().await?;
    let img = match load_rgba(&payload, false) {
        Ok(img) => {img}
        Err(err) => { return Err(tide::Error::new(500,anyhow!("{}",err.to_string())))}
    };
    drop(payload);

    let (out_data, _, _) = match encode_rgba(img.as_ref(), &ravif::Config {
        quality: opts.quality,
        alpha_quality: ((opts.quality + 100.) / 2.).min(opts.quality + opts.quality / 4. + 2.),
        speed: opts.speed,
        premultiplied_alpha: false,
        color_space: ColorSpace::YCbCr,
        threads: 0
    }) {
        Ok(result) => {result}
        Err(err) => {return Err(tide::Error::new(500,anyhow!("{}",err.to_string())))}
    };
    let mut response = Response::new(200);
    response.set_body(out_data);
    response.set_content_type("image/avif");
    Ok(response)
}

type BoxError = Box<dyn std::error::Error + Send + Sync>;

fn load_rgba(mut data: &[u8], premultiplied_alpha: bool) -> Result<ImgVec<RGBA8>, BoxError> {
    use rgb::FromSlice;

    let mut img = if data.get(0..4) == Some(&[0x89,b'P',b'N',b'G']) {
        let img = lodepng::decode32(data)?;
        ImgVec::new(img.buffer, img.width, img.height)
    } else {
        let mut jecoder = jpeg_decoder::Decoder::new(&mut data);
        let pixels = jecoder.decode()?;
        let info = jecoder.info().ok_or("Error reading JPEG info")?;
        use jpeg_decoder::PixelFormat::*;
        let buf: Vec<_> = match info.pixel_format {
            L8 => {
                pixels.iter().copied().map(|g| RGBA8::new(g,g,g,255)).collect()
            },
            RGB24 => {
                let rgb = pixels.as_rgb();
                rgb.iter().map(|p| p.alpha(255)).collect()
            },
            CMYK32 => return Err("CMYK JPEG is not supported. Please convert to PNG first".into()),
        };
        ImgVec::new(buf, info.width.into(), info.height.into())
    };
    if premultiplied_alpha {
        img.pixels_mut().for_each(|px| {
            px.r = (px.r as u16 * px.a as u16 / 255) as u8;
            px.g = (px.g as u16 * px.a as u16 / 255) as u8;
            px.b = (px.b as u16 * px.a as u16 / 255) as u8;
        });
    }
    Ok(img)
}
