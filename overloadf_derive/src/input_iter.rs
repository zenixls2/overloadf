use crate::fn_struct;

pub struct InputIter {
    pub index: usize,
    pub input_types: Vec<syn::Type>,
    pub param_assign: Vec<syn::Pat>,
    pub default_values: Vec<Option<fn_struct::Assign>>,
    pub end: bool,
}

impl InputIter {
    pub fn new(a: Vec<syn::Type>, b: Vec<syn::Pat>, c: Vec<Option<fn_struct::Assign>>) -> Self {
        Self {
            index: a.len(),
            input_types: a,
            param_assign: b,
            default_values: c,
            end: false,
        }
    }
}

impl Iterator for InputIter {
    type Item = (Vec<syn::Type>, Vec<syn::Pat>, Vec<proc_macro2::TokenStream>);
    fn next(&mut self) -> Option<Self::Item> {
        let mut inputs = vec![];
        let mut params = vec![];
        let mut defaults = vec![];
        if self.end {
            return None;
        }
        let mut i = 0;
        loop {
            if i < self.index {
                inputs.push(self.input_types[i].clone());
                params.push(self.param_assign[i].clone());
                i += 1;
            } else {
                break;
            }
        }
        i = self.index;
        loop {
            if i < self.default_values.len() {
                if let Some(default) = &self.default_values[i] {
                    let p = &self.param_assign[i];
                    defaults.push(quote!(let #p #default;));
                } else {
                    inputs.push(self.input_types[i].clone());
                    params.push(self.param_assign[i].clone());
                }
                i += 1;
            } else {
                break;
            }
        }
        loop {
            if self.index > 0 {
                self.index -= 1;
                if self.default_values[self.index].is_some() {
                    break;
                }
            } else {
                self.end = true;
                break;
            }
        }
        Some((inputs, params, defaults))
    }
}
