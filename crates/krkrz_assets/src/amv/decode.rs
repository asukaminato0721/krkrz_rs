//! AJPM entropy reconstruction. Block order and shared DC predictors follow
//! xmoezzz/amv_decoder's format research; scaling is checked against AlphaMovie.
use super::{AlphaEncoding, Movie};
use crate::media::Image;
use anyhow::{Context, Result, bail, ensure};
use std::io::Read;
use super::tables::*;

const ZIGZAG: [usize; 64] = [0,1,8,16,9,2,3,10,17,24,32,25,18,11,4,5,12,19,26,33,40,48,41,34,27,20,13,6,7,14,21,28,35,42,49,56,57,50,43,36,29,22,15,23,30,37,44,51,58,59,52,45,38,31,39,46,53,60,61,54,47,55,62,63];
struct Bits<'a> { bytes: &'a [u8], position: usize }
impl Bits<'_> {
    fn read(&mut self, n: u8) -> Result<i32> {
        ensure!(self.position + n as usize <= self.bytes.len()*8, "truncated AlphaMovie entropy stream");
        let mut v=0;
        for _ in 0..n { v=(v<<1)|((self.bytes[self.position/8]>>(7-self.position%8))&1) as i32; self.position+=1; }
        Ok(v)
    }
    fn signed(&mut self, n: u8) -> Result<i32> {
        if n==0 { return Ok(0); }
        ensure!(n<=11,"invalid AlphaMovie coefficient size");
        let v=self.read(n)?;
        Ok(if v < 1 << (n-1) { v-((1<<n)-1) } else { v })
    }
    fn symbol(&mut self, counts: &[u8;16], values: &[u8]) -> Result<u8> {
        let (mut code,mut first,mut index)=(0,0,0);
        for &count in counts {
            code=(code<<1)|self.read(1)?;
            if code>=first && code<first+count as i32 { return Ok(values[index+(code-first) as usize]); }
            index+=count as usize; first=(first+count as i32)<<1;
        }
        bail!("invalid AlphaMovie Huffman code")
    }
    fn block(&mut self, chroma: bool, predictor: &mut i32, quant: &[u16;64]) -> Result<[u8;64]> {
        let (dc_lengths,dc_values,ac_lengths,ac_values)=if chroma {
            (&CHROMA_DC_CODE_LENGTHS,&CHROMA_DC_VALUES,&CHROMA_AC_CODE_LENGTHS,&CHROMA_AC_VALUES)
        } else { (&LUMA_DC_CODE_LENGTHS,&LUMA_DC_VALUES,&LUMA_AC_CODE_LENGTHS,&LUMA_AC_VALUES) };
        let size=self.symbol(dc_lengths,dc_values)?;
        *predictor=predictor.checked_add(self.signed(size)?).context("AlphaMovie DC predictor overflow")?;
        let mut coeff=[0i16;64];
        coeff[0]=(*predictor).try_into().context("AlphaMovie DC coefficient overflow")?;
        let mut k=1;
        while k<64 {
            let symbol=self.symbol(ac_lengths,ac_values)?;
            if symbol==0 { break; }
            if symbol==0xf0 { k+=16; ensure!(k<=64,"AlphaMovie AC run exceeds block"); continue; }
            k+=(symbol>>4) as usize;
            ensure!(k<64 && symbol&15!=0,"invalid AlphaMovie AC run");
            coeff[ZIGZAG[k]]=self.signed(symbol&15)? as i16;
            k+=1;
        }
        let mut out=[0;64];
        super::idct::dequantize_and_idct_block_8x8(&coeff,quant,8,&mut out);
        Ok(out)
    }
}
impl Movie {
    /// Decode the packet's own rectangle, without carrying pixels from an
    /// earlier frame. Empty packets contain no bitmap.
    pub fn decode_packet(&self, index: usize) -> Result<Option<Image>> {
        let p=self.packets.get(index).context("AlphaMovie packet index out of range")?;
        if p.width==0 { return Ok(None); }
        let width=p.width as usize; let height=p.height as usize;
        let tables:Vec<[u16;64]>=self.quantization.iter().map(|q|std::array::from_fn(|i|q[i] as u16)).collect();
        let alpha=if let Some(compressed)=self.compressed_alpha(index)? {
            let mut alpha=Vec::new();
            flate2::read::ZlibDecoder::new(compressed).take((width*height+1) as u64).read_to_end(&mut alpha)?;
            ensure!(alpha.len()==width*height,"AlphaMovie alpha plane size mismatch");
            Some(alpha)
        } else {None};
        let mut bits=Bits{bytes:self.entropy(index)?,position:0};
        let (mut chroma,mut luma)=(0,0);
        let mut rgba=vec![0;width*height*4];
        for my in (0..height).step_by(16) { for mx in (0..width).step_by(16) {
            let cb=bits.block(true,&mut chroma,&tables[1])?;
            let cr=bits.block(true,&mut chroma,&tables[1])?;
            let mut ys=[[0;64];4]; for block in &mut ys {*block=bits.block(false,&mut luma,&tables[0])?;}
            let mut alphas=[[255;64];4];
            if self.header.alpha_encoding==AlphaEncoding::Dct {
                for block in &mut alphas {*block=bits.block(false,&mut luma,&tables[2])?;}
            }
            for y in 0..16 { for x in 0..16 {
                let block=(y/8)*2+x/8; let at=(y%8)*8+x%8;
                let yy=ys[block][at] as f64; let u=cb[(y/2)*8+x/2] as f64-128.; let v=cr[(y/2)*8+x/2] as f64-128.;
                let pixel=(my+y)*width+mx+x; let out=&mut rgba[pixel*4..pixel*4+4];
                out[0]=(yy+1.402*v).round().clamp(0.,255.) as u8;
                out[1]=(yy-0.344136*u-0.714136*v).round().clamp(0.,255.) as u8;
                out[2]=(yy+1.772*u).round().clamp(0.,255.) as u8;
                out[3]=alpha.as_ref().map_or(alphas[block][at],|a|a[pixel]);
            }}
        }}
        Ok(Some(Image{width:width as u32,height:height as u32,rgba}))
    }
}
