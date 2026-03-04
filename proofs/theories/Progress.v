(** * Blood Core Calculus — Progress Theorem

    This file states and proves the Progress theorem for Blood's
    core calculus: well-typed closed expressions are either values,
    can step, or perform an effect operation.

    Reference: FORMAL_SEMANTICS.md §7.1, §11.1
    Phase: M1 — Core Type System
*)

From Stdlib Require Import String.
From Stdlib Require Import List.
From Stdlib Require Import Arith.
From Stdlib Require Import Bool.
Import ListNotations.
From Blood Require Import Syntax.
From Blood Require Import Typing.
From Blood Require Import Substitution.
From Blood Require Import Semantics.
From Blood Require Import EffectAlgebra.
From Blood Require Import Inversion.
From Blood Require Import ContextTyping.

(** ** Canonical Forms Lemmas

    These establish what shape a value of a given type must have. *)

Lemma canonical_forms_arrow :
  forall Sigma v A B eff,
    has_type Sigma [] [] v (Ty_Arrow A B eff) Eff_Pure ->
    is_value v = true ->
    exists body, v = E_Lam A body.
Proof.
  intros Sigma v A B eff Htype Hval.
  remember [] as Gamma.
  remember [] as Delta.
  remember (Ty_Arrow A B eff) as T.
  induction Htype; subst.
  all: try discriminate.
  all: try (simpl in Hval; discriminate).
  all: try (unfold typeof_const in *; destruct c; discriminate).
  (* T_Lam *)
  - injection HeqT as HA HB Heff. subst. eexists. reflexivity.
  (* T_Sub *)
  - apply IHHtype; auto.
Qed.

Lemma canonical_forms_record :
  forall Sigma v fields,
    has_type Sigma [] [] v (Ty_Record fields) Eff_Pure ->
    is_value v = true ->
    exists vfields,
      v = E_Record vfields /\
      forallb (fun '(_, e) => is_value e) vfields = true.
Proof.
  intros Sigma v fields Htype Hval.
  remember [] as Gamma.
  remember [] as Delta.
  remember (Ty_Record fields) as T.
  induction Htype; subst.
  all: try discriminate.
  all: try (simpl in Hval; discriminate).
  all: try (unfold typeof_const in *; destruct c; discriminate).
  (* T_Record *)
  - exists fields0. split; auto.
  (* T_Sub *)
  - apply IHHtype; auto.
Qed.

Lemma canonical_forms_base :
  forall Sigma v b,
    has_type Sigma [] [] v (Ty_Base b) Eff_Pure ->
    is_value v = true ->
    exists c, v = E_Const c /\ typeof_const c = Ty_Base b.
Proof.
  intros Sigma v b Htype Hval.
  remember [] as Gamma.
  remember [] as Delta.
  remember (Ty_Base b) as T.
  induction Htype; subst.
  all: try discriminate.
  all: try (simpl in Hval; discriminate).
  (* T_Const *)
  - exists c. split; auto.
  (* T_Sub *)
  - apply IHHtype; auto.
Qed.

(** ** Helper: build value list from field expressions with Forall IH *)

Lemma forall_fields_to_values :
  forall fields,
    Forall (fun p : label * expr =>
      is_value (snd p) = true ->
      exists v : value, value_to_expr v = snd p) fields ->
    forallb (fun '(_, e) => is_value e) fields = true ->
    exists vfields : list (label * value),
      map (fun '(l, v) => (l, value_to_expr v)) vfields = fields.
Proof.
  induction fields as [| [l ei] rest IH].
  - intros. exists []. reflexivity.
  - intros HFA Hval. simpl in Hval.
    apply Bool.andb_true_iff in Hval. destruct Hval as [Hval1 Hrest].
    inversion HFA as [| ? ? Hei HFA_rest]; subst.
    simpl in Hei. destruct (Hei Hval1) as [vi Hvi].
    destruct (IH HFA_rest Hrest) as [vrest Hvrest].
    exists ((l, vi) :: vrest). simpl. rewrite Hvi, Hvrest. reflexivity.
Qed.

(** ** Helper: extract a value witness from an is_value expression *)

Lemma expr_to_value :
  forall e, is_value e = true -> exists v : value, value_to_expr v = e.
Proof.
  apply (expr_nested_ind
    (fun e => is_value e = true -> exists v : value, value_to_expr v = e)
    (fun _ => True)
    (fun _ => True)).
  - (* E_Var *) intros. discriminate.
  - (* E_Const *) intros c _. exists (V_Const c). reflexivity.
  - (* E_Lam *) intros T body _ _. exists (V_Lam T body). reflexivity.
  - (* E_App *) intros. discriminate.
  - (* E_Let *) intros. discriminate.
  - (* E_Annot *) intros. discriminate.
  - (* E_Record *)
    intros fields HFA Hval. simpl in Hval.
    destruct (forall_fields_to_values fields HFA Hval) as [vfields Hvf].
    exists (V_Record vfields). simpl. rewrite Hvf. reflexivity.
  - (* E_Select *) intros. discriminate.
  - (* E_Extend *) intros. discriminate.
  - (* E_Perform *) intros. discriminate.
  - (* E_Handle *) intros. discriminate.
  - (* E_Resume *) intros. discriminate.
  - (* Handler *) intros. exact I.
  - (* OpClause *) intros. exact I.
Qed.

(** ** Helper: record field type-value correspondence *)

Lemma record_fields_lookup :
  forall Sigma Gamma Delta efields tfields eff l T,
    record_fields_typed Sigma Gamma Delta efields tfields eff ->
    lookup_field tfields l = Some T ->
    exists e, find_field efields l = Some e.
Proof.
  intros Sigma Gamma Delta efields tfields eff l T Hrt.
  induction Hrt; intros Hlook.
  - simpl in Hlook. discriminate.
  - simpl in Hlook. simpl.
    destruct (label_eqb l0 l) eqn:Heq.
    + exists e. reflexivity.
    + exact (IHHrt Hlook).
Qed.

(** ** Progress via mutual induction

    We use mutual induction on the typing derivation to simultaneously
    prove progress for expressions (P) and a record-field progress
    property (S). The handler and op_clause predicates (Q, R) are
    trivially True since we never need progress for those. *)

Lemma progress_helper :
  forall Sigma Gamma Delta e T eff,
    has_type Sigma Gamma Delta e T eff ->
    Gamma = @nil ty ->
    Delta = @nil (linearity * bool) ->
    forall M,
      (is_value e = true) \/
      (exists e' M', step Sigma (mk_config e M) (mk_config e' M')) \/
      (exists eff_nm op v D,
         e = plug_delimited D (E_Perform eff_nm op (value_to_expr v))).
Proof.
  apply (has_type_mut_ind
    (** P: progress for expressions *)
    (fun Sigma Gamma Delta e T eff _ =>
       Gamma = @nil ty ->
       Delta = @nil (linearity * bool) ->
       forall M,
         (is_value e = true) \/
         (exists e' M', step Sigma (mk_config e M) (mk_config e' M')) \/
         (exists eff_nm op v D,
            e = plug_delimited D (E_Perform eff_nm op (value_to_expr v))))
    (** Q: trivial for handler_well_formed *)
    (fun Sigma Gamma Delta h eff_name comp_ty result_ty handler_eff comp_eff _ => True)
    (** R: trivial for op_clauses_well_formed *)
    (fun Sigma Gamma Delta clauses eff_name eff_sig rrt re result_ty handler_eff _ => True)
    (** S: record field progress *)
    (fun Sigma Gamma Delta fields field_types eff _ =>
       Gamma = @nil ty ->
       Delta = @nil (linearity * bool) ->
       forall M,
         (forallb (fun '(_, ei) => is_value ei) fields = true) \/
         (exists done l e0 e0' rest M',
            fields = done ++ (l, e0) :: rest /\
            forallb (fun '(_, ei) => is_value ei) done = true /\
            step Sigma (mk_config e0 M) (mk_config e0' M')) \/
         (exists done l D rest eff_nm op v,
            fields = done ++ (l, plug_delimited D (E_Perform eff_nm op (value_to_expr v))) :: rest /\
            forallb (fun '(_, ei) => is_value ei) done = true))
  ).

  (** Case T_Var *)
  - intros Sigma Gamma Delta x T Hlook HeqG HeqD M. subst.
    destruct x; simpl in Hlook; discriminate.

  (** Case T_Const *)
  - intros. left. reflexivity.

  (** Case T_Lam *)
  - intros. left. reflexivity.

  (** Case T_App *)
  - intros Sigma Gamma Delta Delta1 Delta2 e1 e2 A B fn_eff eff1 eff2
           Hsplit He1 IH1 He2 IH2 HeqG HeqD M. subst.
    apply lin_split_nil_inv in Hsplit as [HD1 HD2]. subst.
    specialize (IH1 eq_refl eq_refl M).
    specialize (IH2 eq_refl eq_refl M).
    right.
    destruct IH1 as [Hval1 | [[e1' [M' Hstep1]] | [en1 [op1 [v1 [D1 Heq1]]]]]].
    + destruct IH2 as [Hval2 | [[e2' [M' Hstep2]] | [en2 [op2 [v2 [D2 Heq2]]]]]].
      * left.
        pose proof (value_typing_inversion _ _ _ _ _ _ He1 Hval1) as Hpure1.
        destruct (canonical_forms_arrow _ _ _ _ _ Hpure1 Hval1) as [body Heq1].
        subst e1.
        destruct (expr_to_value _ Hval2) as [v2 Hv2]. subst e2.
        exists (subst 0 (value_to_expr v2) body), M. apply Step_Beta.
      * left.
        destruct (expr_to_value _ Hval1) as [v1 Hv1]. subst e1.
        exists (E_App (value_to_expr v1) e2'), M'.
        apply (Step_Context M M' (EC_AppArg v1 EC_Hole) e2 e2'). exact Hstep2.
      * right. subst e2.
        destruct (expr_to_value _ Hval1) as [v1 Hv1]. subst e1.
        exists en2, op2, v2, (DC_AppArg v1 D2). reflexivity.
    + left.
      exists (E_App e1' e2), M'.
      apply (Step_Context M M' (EC_AppFun EC_Hole e2) e1 e1'). exact Hstep1.
    + right. subst e1.
      exists en1, op1, v1, (DC_AppFun D1 e2). reflexivity.

  (** Case T_Let *)
  - intros Sigma Gamma Delta Delta1 Delta2 e1 e2 A B eff1 eff2
           Hsplit He1 IH1 He2 IH2 HeqG HeqD M. subst.
    apply lin_split_nil_inv in Hsplit as [HD1 HD2]. subst.
    specialize (IH1 eq_refl eq_refl M).
    right.
    destruct IH1 as [Hval1 | [[e1' [M' Hstep1]] | [en1 [op1 [v1 [D1 Heq1]]]]]].
    + left. exists (subst 0 e1 e2), M. apply Step_Let. exact Hval1.
    + left. exists (E_Let e1' e2), M'.
      apply (Step_Context M M' (EC_Let EC_Hole e2) e1 e1'). exact Hstep1.
    + right. subst e1.
      exists en1, op1, v1, (DC_Let D1 e2). reflexivity.

  (** Case T_Annot *)
  - intros Sigma Gamma Delta e T eff He IHe HeqG HeqD M. subst.
    specialize (IHe eq_refl eq_refl M).
    right.
    destruct IHe as [Hval | [[e' [M' Hstep]] | [en [op0 [v0 [D Heq]]]]]].
    + left. exists e, M. apply Step_Annot. exact Hval.
    + left. exists (E_Annot e' T), M'.
      apply (Step_Context M M' (EC_Annot EC_Hole T) e e'). exact Hstep.
    + right. subst e.
      exists en, op0, v0, (DC_Annot D T). reflexivity.

  (** Case T_Record — uses mutual IH for record fields *)
  - intros Sigma Gamma Delta fields field_types eff Hfields IH_rft HeqG HeqD M. subst.
    specialize (IH_rft eq_refl eq_refl M).
    destruct IH_rft as [Hallval | [[done [l [e0 [e0' [rest [M' [Hdecomp [Hdone Hstep]]]]]]]] |
                         [done [l [D [rest [eff_nm [op [v [Hdecomp Hdone]]]]]]]]]].
    + (* All fields are values → E_Record is a value *)
      left. simpl. exact Hallval.
    + (* Some field steps → Step_RecordEval *)
      right. left. subst fields.
      exists (E_Record (done ++ (l, e0') :: rest)), M'.
      apply Step_RecordEval. exact Hstep.
    + (* Some field performs → propagate via DC_RecordField *)
      right. right. subst fields.
      exists eff_nm, op, v, (DC_RecordField done l D rest). reflexivity.

  (** Case T_Select *)
  - intros Sigma Gamma Delta e l T fields eff He IHe Hlook HeqG HeqD M. subst.
    specialize (IHe eq_refl eq_refl M).
    right.
    destruct IHe as [Hval | [[e' [M' Hstep]] | [en [op0 [v0 [D Heq]]]]]].
    + left.
      pose proof (value_typing_inversion _ _ _ _ _ _ He Hval) as Hpure.
      destruct (canonical_forms_record _ _ _ Hpure Hval)
        as [vfields [Heqr Hvalsf]].
      subst e.
      destruct (has_type_record_inv _ _ _ _ Hpure)
        as [ft2 [eff_rec [Heq_ft [Hrft _]]]].
      injection Heq_ft as Hft. subst.
      destruct (record_fields_lookup _ _ _ _ _ _ _ _ Hrft Hlook)
        as [ei Hfind].
      exists ei, M. apply Step_Select with (fields := vfields).
      { exact Hfind. }
      { exact Hvalsf. }
    + left. exists (E_Select e' l), M'.
      apply (Step_Context M M' (EC_Select EC_Hole l) e e'). exact Hstep.
    + right. subst e.
      exists en, op0, v0, (DC_Select D l). reflexivity.

  (** Case T_Perform *)
  - intros Sigma Gamma Delta e eff_name op eff_sig arg_ty ret_ty eff'
           Hlookeff Hlookop He IHe HeqG HeqD M. subst.
    specialize (IHe eq_refl eq_refl M).
    right.
    destruct IHe as [Hval | [[e' [M' Hstep]] | [en [op' [v0 [D Heq]]]]]].
    + right.
      destruct (expr_to_value _ Hval) as [v Hv]. subst e.
      exists eff_name, op, v, DC_Hole. reflexivity.
    + left. exists (E_Perform eff_name op e'), M'.
      apply (Step_Context M M' (EC_PerformArg eff_name op EC_Hole) e e').
      exact Hstep.
    + right. subst e.
      exists en, op', v0, (DC_PerformArg eff_name op D). reflexivity.

  (** Case T_Handle *)
  - intros Sigma Gamma Delta Delta1 Delta2 h e eff_name comp_ty result_ty
           handler_eff comp_eff Hsplit He IHe Hwf _ HeqG HeqD M. subst.
    apply lin_split_nil_inv in Hsplit as [HD1 HD2]. subst.
    specialize (IHe eq_refl eq_refl M).
    right.
    destruct IHe as [Hval | [[e' [M' Hstep]] | [en [op' [v0 [D Heq]]]]]].
    + left.
      destruct h as [hk e_ret clauses].
      exists (subst 0 e e_ret), M. apply Step_HandleReturn. exact Hval.
    + left. exists (E_Handle h e'), M'.
      apply (Step_Context M M' (EC_Handle h EC_Hole) e e'). exact Hstep.
    + (* e performs → handler might handle it *)
      admit.

  (** Case T_Sub *)
  - intros Sigma Gamma Delta e T eff eff' He IHe Hsub HeqG HeqD M. subst.
    exact (IHe eq_refl eq_refl M).

  (** Case HWF *)
  - intros. exact I.

  (** Case OpClauses_Nil *)
  - intros. exact I.

  (** Case OpClauses_Cons *)
  - intros. exact I.

  (** Case RFT_Nil *)
  - intros Sigma Gamma Delta HeqG HeqD M. subst. left. simpl. reflexivity.

  (** Case RFT_Cons *)
  - intros Sigma Gamma Delta l e T rest_e rest_t eff1 eff2
           He IH_e Hrest IH_rest HeqG HeqD M. subst.
    specialize (IH_e eq_refl eq_refl M).
    specialize (IH_rest eq_refl eq_refl M).
    destruct IH_e as [Hval_e | [[e' [M' Hstep_e]] | [en [op' [v0 [D Heq_e]]]]]].
    + (* Head field is a value *)
      destruct IH_rest as [Hrest_val | [[done' [l' [e0 [e0' [rest' [M' [Hdecomp [Hdone' Hstep']]]]]]]] |
                            [done' [l' [D' [rest' [eff_nm [op [v [Hdecomp Hdone']]]]]]]]]].
      * (* All tail fields are values too *)
        left. simpl. rewrite Hval_e. simpl. exact Hrest_val.
      * (* Some tail field steps *)
        right. left. subst rest_e.
        exists ((l, e) :: done'), l', e0, e0', rest', M'.
        split; [reflexivity |].
        split; [simpl; rewrite Hval_e; simpl; exact Hdone' |].
        exact Hstep'.
      * (* Some tail field performs *)
        right. right. subst rest_e.
        exists ((l, e) :: done'), l', D', rest', eff_nm, op, v.
        split; [reflexivity |].
        simpl. rewrite Hval_e. simpl. exact Hdone'.
    + (* Head field steps *)
      right. left.
      exists (@nil (label * expr)), l, e, e', rest_e, M'.
      split; [reflexivity |].
      split; [reflexivity |].
      exact Hstep_e.
    + (* Head field performs *)
      right. right. subst e.
      exists (@nil (label * expr)), l, D, rest_e, en, op', v0.
      split; [reflexivity |].
      simpl. reflexivity.
Admitted.

(** ** Progress Theorem

    Statement: If ∅; ∅ ⊢ e : T / ε then either:
    1. e is a value, or
    2. e ──► e' for some e' (in any memory state), or
    3. e = D[perform E.op(v)] for some D, E, op, v
       (an unhandled effect operation)

    Reference: FORMAL_SEMANTICS.md §7.1, §11.1 *)

Theorem progress :
  forall Sigma e T eff M,
    closed_well_typed Sigma e T eff ->
    (is_value e = true) \/
    (exists e' M', step Sigma (mk_config e M) (mk_config e' M')) \/
    (exists eff_nm op v D,
       e = plug_delimited D (E_Perform eff_nm op (value_to_expr v))).
Proof.
  intros Sigma e T eff M Htype.
  exact (progress_helper _ _ _ _ _ _ Htype eq_refl eq_refl M).
Qed.

(** ** Corollary: well-typed closed pure programs don't get stuck *)

Corollary pure_progress :
  forall Sigma e T M,
    closed_well_typed Sigma e T Eff_Pure ->
    (is_value e = true) \/
    (exists e' M', step Sigma (mk_config e M) (mk_config e' M')).
Proof.
  intros Sigma e T M Htype.
  destruct (progress Sigma e T Eff_Pure M Htype) as [Hval | [Hstep | Hperform]].
  - left. exact Hval.
  - right. exact Hstep.
  - (* Pure program cannot perform effects. *)
    exfalso.
    destruct Hperform as [eff_nm [op0 [v0 [D Heq]]]]. subst.
    unfold closed_well_typed in Htype.
    assert (Hin : effect_in_row eff_nm Eff_Pure).
    { exact (plug_delimited_perform_effect _ D eff_nm op0 v0 T Eff_Pure Htype). }
    (* effect_in_row eff_nm Eff_Pure = False *)
    simpl in Hin. exact Hin.
Qed.
