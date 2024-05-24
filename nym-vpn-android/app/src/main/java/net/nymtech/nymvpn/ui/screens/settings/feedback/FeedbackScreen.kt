package net.nymtech.nymvpn.ui.screens.settings.feedback

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.res.vectorResource
import androidx.compose.ui.unit.dp
import net.nymtech.nymvpn.R
import net.nymtech.nymvpn.ui.AppViewModel
import net.nymtech.nymvpn.ui.common.buttons.surface.SelectionItem
import net.nymtech.nymvpn.ui.common.buttons.surface.SurfaceSelectionGroupButton
import net.nymtech.nymvpn.util.scaledHeight
import net.nymtech.nymvpn.util.scaledWidth

@Composable
fun FeedbackScreen(appViewModel: AppViewModel) {
	val context = LocalContext.current

// 	AnimatedVisibility(showErrorReportingDialog) {
// 		AlertDialog(
// 			containerColor = CustomColors.snackBarBackgroundColor,
// 			onDismissRequest = { showErrorReportingDialog = false },
// 			confirmButton = {
// 				TextButton(
// 					onClick = {
// 						showErrorReportingDialog = false
// 						appViewModel.onErrorReportingSelected()
// 					},
// 				) {
// 					Text(text = stringResource(R.string.okay))
// 				}
// 			},
// 			dismissButton = {
// 				TextButton(onClick = { showErrorReportingDialog = false }) {
// 					Text(text = stringResource(R.string.cancel))
// 				}
// 			},
// 			title = {
// 				Text(
// 					text = stringResource(R.string.error_reporting),
// 					color = CustomColors.snackbarTextColor,
// 				)
// 			},
// 			text = {
// 				Text(
// 					text = stringResource(R.string.error_reporting_alert),
// 					color = CustomColors.snackbarTextColor,
// 				)
// 			},
// 		)
// 	}

	Column(
		horizontalAlignment = Alignment.Start,
		verticalArrangement = Arrangement.spacedBy(24.dp, Alignment.Top),
		modifier =
		Modifier
			.verticalScroll(rememberScrollState())
			.fillMaxSize()
			.padding(top = 24.dp.scaledHeight())
			.padding(horizontal = 24.dp.scaledWidth()),
	) {
		SurfaceSelectionGroupButton(
			listOf(
				SelectionItem(
					leadingIcon = ImageVector.vectorResource(R.drawable.github),
					title = { Text(stringResource(R.string.open_github), style = MaterialTheme.typography.bodyLarge.copy(MaterialTheme.colorScheme.onSurface)) },
					onClick = {
						appViewModel.openWebPage(
							context.getString(R.string.github_issues_url),
							context,
						)
					},
				),
			),
		)
		SurfaceSelectionGroupButton(
			listOf(
				SelectionItem(
					leadingIcon = ImageVector.vectorResource(R.drawable.send),
					title = { Text(stringResource(R.string.send_feedback), style = MaterialTheme.typography.bodyLarge.copy(MaterialTheme.colorScheme.onSurface)) },
					onClick = {
						appViewModel.openWebPage(context.getString(R.string.contact_url), context)
					},
				),
			),
		)
		SurfaceSelectionGroupButton(
			listOf(
				SelectionItem(
					leadingIcon = ImageVector.vectorResource(R.drawable.matrix),
					title = { Text(stringResource(R.string.join_matrix), style = MaterialTheme.typography.bodyLarge.copy(MaterialTheme.colorScheme.onSurface)) },
					onClick = {
						appViewModel.openWebPage(context.getString(R.string.matrix_url), context)
					},
				),
			),
		)
		SurfaceSelectionGroupButton(
			listOf(
				SelectionItem(
					leadingIcon = ImageVector.vectorResource(R.drawable.discord),
					title = { Text(stringResource(R.string.join_discord), style = MaterialTheme.typography.bodyLarge.copy(MaterialTheme.colorScheme.onSurface)) },
					onClick = {
						appViewModel.openWebPage(context.getString(R.string.discord_url), context)
					},
				),
			),
		)
	}
}
