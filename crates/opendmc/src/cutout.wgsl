// `--look rt`: the G-buffer pass of a cut-out (see rt.rs `Cutout`). The
// material is opaque, so Bevy 0.18.1 keeps it out of the forward pass, and
// the alpha test happens here instead.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    if pbr_input.material.base_color.a < 0.5 {
        discard;
    }
    pbr_input.material.base_color.a = 1.0;
    return deferred_output(in, pbr_input);
}
