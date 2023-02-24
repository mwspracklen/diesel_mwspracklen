use proc_macro2::TokenStream;
use syn::spanned::Spanned;
use syn::DeriveInput;

use field::Field;
use model::Model;
use util::wrap_in_dummy_mod;

pub fn derive(item: DeriveInput) -> TokenStream {
    let model = Model::from_item(&item, false);

    let (_, ty_generics, _) = item.generics.split_for_impl();

    let mut generics = item.generics.clone();
    generics
        .params
        .push(parse_quote!(__DB: diesel::backend::Backend));

    for embed_field in model.fields().iter().filter(|f| f.embed()) {
        let embed_ty = &embed_field.ty;
        generics
            .where_clause
            .get_or_insert_with(|| parse_quote!(where))
            .predicates
            .push(parse_quote!(#embed_ty: Selectable<__DB>));
    }

    let (impl_generics, _, where_clause) = generics.split_for_impl();

    let struct_name = &item.ident;

    let field_columns_ty = model
        .fields()
        .iter()
        .map(|f| field_column_ty(f, &model))
        .collect::<Vec<_>>();
    let field_columns_inst = model.fields().iter().map(|f| field_column_inst(f, &model));

    let field_check_bound = model
        .fields()
        .iter()
        .zip(&field_columns_ty)
        .filter(|(f, _)| !f.embed())
        .flat_map(|(f, ty)| {
            model
                .check_for_backend
                .as_ref()
                .into_iter()
                .flat_map(move |d| {
                    let field_ty = to_field_ty_bound(&f.ty);
                    let span = field_ty.span();
                    d.iter().map(move |b| {
                        quote::quote_spanned! {span =>
                            #field_ty: diesel::deserialize::FromSqlRow<diesel::dsl::SqlTypeOf<#ty>, #b>
                        }
                    })
                })
        });

    wrap_in_dummy_mod(quote! {
        use diesel::expression::Selectable;

        impl #impl_generics Selectable<__DB>
            for #struct_name #ty_generics
        #where_clause
        {
            type SelectExpression = (#(#field_columns_ty,)*);

            fn construct_selection() -> Self::SelectExpression {
                (#(#field_columns_inst,)*)
            }
        }

        fn _check_field_compatibility()
        where
            #(#field_check_bound,)*
        {

        }
    })
}

fn to_field_ty_bound(field_ty: &syn::Type) -> Option<TokenStream> {
    match field_ty {
        syn::Type::Path(p) => {
            if let syn::PathArguments::AngleBracketed(ref args) =
                p.path.segments.last().unwrap().arguments
            {
                let lt = args
                    .args
                    .iter()
                    .filter_map(|f| {
                        if let syn::GenericArgument::Lifetime(lt) = f {
                            Some(lt)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                if lt.is_empty() {
                    Some(quote::quote! {
                        #field_ty
                    })
                } else if lt.len() == args.args.len() {
                    Some(quote::quote! {
                        for<#(#lt,)*> #field_ty
                    })
                } else {
                    // type parameters are not supported for checking
                    // for now
                    None
                }
            } else {
                Some(quote::quote! {
                    #field_ty
                })
            }
        }
        syn::Type::Reference(_r) => {
            // references are not supported for checking for now
            //
            // (How ever you can even have references in a `Queryable` struct anyway)
            None
        }
        field_ty => Some(quote::quote! {
            #field_ty
        }),
    }
}

fn field_column_ty(field: &Field, model: &Model) -> TokenStream {
    if let Some(ref select_expression_type) = field.select_expression_type {
        let ty = &select_expression_type.item;
        quote!(#ty)
    } else if field.embed() {
        let embed_ty = &field.ty;
        quote!(<#embed_ty as Selectable<__DB>>::SelectExpression)
    } else {
        let table_name = model.table_name();
        let column_name = field.column_name();
        quote!(#table_name::#column_name)
    }
}

fn field_column_inst(field: &Field, model: &Model) -> TokenStream {
    if let Some(ref select_expression) = field.select_expression {
        let expr = &select_expression.item;
        let span = expr.span();
        quote::quote_spanned!(span => #expr)
    } else if field.embed() {
        let embed_ty = &field.ty;
        quote!(<#embed_ty as Selectable<__DB>>::construct_selection())
    } else {
        let table_name = model.table_name();
        let column_name = field.column_name();
        quote!(#table_name::#column_name)
    }
}
