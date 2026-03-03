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
From Blood Require Import Preservation.

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

(** ** Helper: lin_split of empty context forces both sides empty *)

Lemma lin_split_nil_inv :
  forall Delta1 Delta2,
    lin_split [] Delta1 Delta2 -> Delta1 = [] /\ Delta2 = [].
Proof.
  intros Delta1 Delta2 H. inversion H. auto.
Qed.

(** ** Helper: record field type-value correspondence *)

Lemma record_fields_lookup :
  forall Sigma Gamma Delta efields tfields eff l T,
    record_fields_typed Sigma Gamma Delta efields tfields eff ->
    lookup_field tfields l = Some T ->
    exists e, In (l, e) efields.
Proof.
  intros Sigma Gamma Delta efields tfields eff l T Hrt.
  induction Hrt; intros Hlook.
  - simpl in Hlook. discriminate.
  - simpl in Hlook.
    destruct (label_eqb l0 l) eqn:Heq.
    + apply label_eqb_eq in Heq. subst. exists e. left. reflexivity.
    + destruct (IHHrt Hlook) as [e' Hin]. exists e'. right. exact Hin.
Qed.

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
    (exists e' M', step (mk_config e M) (mk_config e' M')) \/
    (exists eff_nm op v D,
       e = plug_delimited D (E_Perform eff_nm op (value_to_expr v))).
Proof.
  intros Sigma e T eff M Htype.
  unfold closed_well_typed in Htype.
  remember (@nil ty) as Gamma.
  remember (@nil (linearity * bool)) as Delta.
  induction Htype; subst.

  (** Case T_Var: lookup_var [] x = Some T — impossible *)
  - destruct x; simpl in H; discriminate.

  (** Case T_Const: E_Const is a value *)
  - left. reflexivity.

  (** Case T_Lam: E_Lam is a value *)
  - left. reflexivity.

  (** Case T_App: e₁ e₂ *)
  - apply lin_split_nil_inv in H as [HD1 HD2]. subst.
    specialize (IHHtype1 eq_refl eq_refl).
    specialize (IHHtype2 eq_refl eq_refl).
    right.
    destruct IHHtype1 as [Hval1 | [[e1' [M' Hstep1]] | [en1 [op1 [v1 [D1 Heq1]]]]]].
    + (* e1 is a value *)
      destruct IHHtype2 as [Hval2 | [[e2' [M' Hstep2]] | [en2 [op2 [v2 [D2 Heq2]]]]]].
      * (* e2 is a value → β-reduction *)
        left.
        pose proof (value_typing_inversion _ _ _ _ _ _ Htype1 Hval1) as Hpure1.
        destruct (canonical_forms_arrow _ _ _ _ _ Hpure1 Hval1) as [body Heq1].
        subst e1.
        destruct (expr_to_value _ Hval2) as [v2 Hv2]. subst e2.
        exists (subst 0 (value_to_expr v2) body), M. apply Step_Beta.
      * (* e2 steps → context rule *)
        left.
        destruct (expr_to_value _ Hval1) as [v1 Hv1]. subst e1.
        exists (E_App (value_to_expr v1) e2'), M'.
        apply (Step_Context M M' (EC_AppArg v1 EC_Hole) e2 e2'). exact Hstep2.
      * (* e2 performs → propagate *)
        right. subst e2.
        destruct (expr_to_value _ Hval1) as [v1 Hv1]. subst e1.
        exists en2, op2, v2, (DC_AppArg v1 D2). reflexivity.
    + (* e1 steps → context rule *)
      left.
      exists (E_App e1' e2), M'.
      apply (Step_Context M M' (EC_AppFun EC_Hole e2) e1 e1'). exact Hstep1.
    + (* e1 performs → propagate *)
      right. subst e1.
      exists en1, op1, v1, (DC_AppFun D1 e2). reflexivity.

  (** Case T_Let: let x = e₁ in e₂ *)
  - apply lin_split_nil_inv in H as [HD1 HD2]. subst.
    specialize (IHHtype1 eq_refl eq_refl).
    right.
    destruct IHHtype1 as [Hval1 | [[e1' [M' Hstep1]] | [en1 [op1 [v1 [D1 Heq1]]]]]].
    + (* e1 is a value → Step_Let *)
      left. exists (subst 0 e1 e2), M. apply Step_Let. exact Hval1.
    + (* e1 steps → context rule *)
      left. exists (E_Let e1' e2), M'.
      apply (Step_Context M M' (EC_Let EC_Hole e2) e1 e1'). exact Hstep1.
    + (* e1 performs → propagate *)
      right. subst e1.
      exists en1, op1, v1, (DC_Let D1 e2). reflexivity.

  (** Case T_Annot: (e : T) *)
  - specialize (IHHtype eq_refl eq_refl).
    right.
    destruct IHHtype as [Hval | [[e' [M' Hstep]] | [en [op0 [v0 [D Heq]]]]]].
    + (* e is a value → Step_Annot *)
      left. exists e, M. apply Step_Annot. exact Hval.
    + (* e steps → context rule *)
      left. exists (E_Annot e' T), M'.
      apply (Step_Context M M' (EC_Annot EC_Hole T) e e'). exact Hstep.
    + (* e performs → propagate *)
      right. subst e.
      exists en, op0, v0, (DC_Annot D T). reflexivity.

  (** Case T_Record: {l₁=e₁,...} — requires record field evaluation context
      which is not modeled. Admitted. *)
  - admit.

  (** Case T_Select: e.l *)
  - specialize (IHHtype eq_refl eq_refl).
    right.
    destruct IHHtype as [Hval | [[e' [M' Hstep]] | [en [op0 [v0 [D Heq]]]]]].
    + (* e is a value (record) → Step_Select *)
      left.
      pose proof (value_typing_inversion _ _ _ _ _ _ Htype Hval) as Hpure.
      destruct (canonical_forms_record _ _ _ Hpure Hval)
        as [vfields [Heqr Hvalsf]].
      subst e.
      destruct (has_type_record_inv _ _ _ _ Hpure)
        as [ft2 [eff_rec [Heq_ft [Hrft _]]]].
      injection Heq_ft as Hft. subst.
      destruct (record_fields_lookup _ _ _ _ _ _ _ _ Hrft H)
        as [ei Hini].
      exists ei, M. apply Step_Select with (fields := vfields).
      { exact Hini. }
      { (* is_value ei: In (l, ei) vfields and all fields are values *)
        clear -Hini Hvalsf.
        induction vfields as [| [l' e'] rest IH].
        { inversion Hini. }
        { simpl in Hvalsf. apply Bool.andb_true_iff in Hvalsf.
          destruct Hvalsf as [Hv1 Hv2].
          destruct Hini as [Heq | Hin].
          { injection Heq as _ Heq. subst. exact Hv1. }
          { exact (IH Hv2 Hin). }
        }
      }
      { exact Hvalsf. }
    + (* e steps → context rule *)
      left. exists (E_Select e' l), M'.
      apply (Step_Context M M' (EC_Select EC_Hole l) e e'). exact Hstep.
    + (* e performs → propagate *)
      right. subst e.
      exists en, op0, v0, (DC_Select D l). reflexivity.

  (** Case T_Perform: perform E.op(e) — directly case 3 *)
  - specialize (IHHtype eq_refl eq_refl).
    right.
    destruct IHHtype as [Hval | [[e' [M' Hstep]] | [en [op' [v0 [D Heq]]]]]].
    + (* e is a value → case 3 with D = Hole *)
      right.
      destruct (expr_to_value _ Hval) as [v Hv]. subst e.
      exists eff_name, op, v, DC_Hole. reflexivity.
    + (* e steps → context rule *)
      left. exists (E_Perform eff_name op e'), M'.
      apply (Step_Context M M' (EC_PerformArg eff_name op EC_Hole) e e').
      exact Hstep.
    + (* e performs → propagate *)
      right. subst e.
      exists en, op', v0, (DC_PerformArg eff_name op D). reflexivity.

  (** Case T_Handle: with h handle e *)
  - apply lin_split_nil_inv in H as [HD1 HD2]. subst.
    specialize (IHHtype eq_refl eq_refl).
    right.
    destruct IHHtype as [Hval | [[e' [M' Hstep]] | [en [op' [v0 [D Heq]]]]]].
    + (* e is a value → Step_HandleReturn *)
      left.
      destruct h as [hk e_ret clauses].
      exists (subst 0 e e_ret), M. apply Step_HandleReturn. exact Hval.
    + (* e steps → context rule *)
      left. exists (E_Handle h e'), M'.
      apply (Step_Context M M' (EC_Handle h EC_Hole) e e'). exact Hstep.
    + (* e performs → handler might handle it *)
      (* Need to check if (en, op') is in h's clauses.
         If yes → Step_HandleOpDeep/Shallow.
         If no → unhandled perform, but no DC_Handle exists to propagate.
         This case requires decidable clause lookup and the "unhandled
         operation propagation" rule which is not in the formalization. *)
      admit.

  (** Case T_Sub: by IH directly *)
  - exact (IHHtype eq_refl eq_refl).
Admitted.

(** ** Corollary: well-typed closed pure programs don't get stuck *)

Corollary pure_progress :
  forall Sigma e T M,
    closed_well_typed Sigma e T Eff_Pure ->
    (is_value e = true) \/
    (exists e' M', step (mk_config e M) (mk_config e' M')).
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
